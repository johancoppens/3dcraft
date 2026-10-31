//! Hand-written files that are wrong in one way each: the error names
//! what is wrong, and nothing panics. Round trips of whole bodies are in
//! `crates/io/tests/step_import.rs`, which has the exporter.

use super::*;
use craft3d_math::Portable;
use craft3d_math::Tolerance;

const HEAD: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n";
const TAIL: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

/// Units in millimetres, and a solid whose one face is bounded by one
/// edge on curve `#30`, which `curve` defines (with ids from 30 up).
fn file(curve: &str, units: bool) -> String {
    let context = if units {
        "#1=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
         #2=( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );\n\
         #3=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#1,'distance_accuracy_value','');\n\
         #4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#3)) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2)) REPRESENTATION_CONTEXT('','') );\n"
    } else {
        "#4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) REPRESENTATION_CONTEXT('','') );\n"
    };
    format!(
        "{HEAD}{context}\
         #5=ADVANCED_BREP_SHAPE_REPRESENTATION('part',(#10),#4);\n\
         #10=MANIFOLD_SOLID_BREP('solid',#11);\n\
         #11=CLOSED_SHELL('',(#12));\n\
         #12=ADVANCED_FACE('',(#13),#20,.T.);\n\
         #13=FACE_OUTER_BOUND('',#14,.T.);\n\
         #14=EDGE_LOOP('',(#15));\n\
         #15=ORIENTED_EDGE('',*,*,#16,.T.);\n\
         #16=EDGE_CURVE('',#17,#17,#30,.T.);\n\
         #17=VERTEX_POINT('',#18);\n\
         #18=CARTESIAN_POINT('',(1.,0.,0.));\n\
         #19=CARTESIAN_POINT('',(0.,0.,0.));\n\
         #20=PLANE('',#21);\n\
         #21=AXIS2_PLACEMENT_3D('',#19,$,$);\n\
         {curve}{TAIL}"
    )
}

fn options() -> Options {
    Options {
        metres_per_unit: 1e-3,
        tolerance: Tolerance::default(),
        loosest: 1000.0 * (Tolerance::default()).linear,
    }
}

fn first(text: &str) -> Result<Imported, ImportError> {
    let parsed = parse(text.as_bytes())?;
    parsed.build(0, &options())
}

fn error(text: &str) -> String {
    match first(text) {
        Ok(_) => panic!("read without an error"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_closed_circle_reads_and_stays_a_circle() {
    // A plane face bounded by a whole circle: a sheet in a solid's place,
    // which sewing refuses (an edge used once, no free edge to sew it to)
    // with the body's name and the edge's (the check refused it with
    // "used 1 times" before sewing ran on every import, 2026-10-22).
    let text = file("#30=CIRCLE('',#21,1.);\n", true);
    let e = error(&text);
    assert!(e.contains("`solid`"), "{e}");
    assert!(e.contains("edge #16 bounds only one face"), "{e}");
}

#[test]
fn what_is_not_read_is_named() {
    let e = error(&file("#30=HYPERBOLA('',#21,1.,2.);\n", true));
    assert!(e.contains("#30 HYPERBOLA") && e.contains("not read"), "{e}");
    let e = error(&file(
        "#30=SURFACE_CURVE('',#31,(),.CURVE_3D.);\n#31=PARABOLA('',#21,1.);\n",
        true,
    ));
    assert!(e.contains("#31 PARABOLA"), "{e}");
}

#[test]
fn a_file_without_units_is_refused() {
    let e = error(&file("#30=CIRCLE('',#21,1.);\n", false));
    assert!(e.contains("no units") || e.contains("units"), "{e}");
}

#[test]
fn a_chain_of_curves_in_a_circle_ends() {
    // SURFACE_CURVE whose 3D curve is itself, through another.
    let e = error(&file(
        "#30=SURFACE_CURVE('',#31,(),.CURVE_3D.);\n#31=SEAM_CURVE('',#30,(),.CURVE_3D.);\n",
        true,
    ));
    assert!(e.contains("more than"), "{e}");
}

#[test]
fn wrong_kinds_of_references_are_named() {
    // The vertex of an edge is a point, not a VERTEX_POINT.
    let text = file("#30=CIRCLE('',#21,1.);\n", true).replace(
        "#16=EDGE_CURVE('',#17,#17,#30,.T.);",
        "#16=EDGE_CURVE('',#18,#18,#30,.T.);",
    );
    let e = error(&text);
    assert!(
        e.contains("#18 is a CARTESIAN_POINT") && e.contains("VERTEX_POINT"),
        "{e}"
    );
    // A face whose surface is a curve.
    let text = file("#30=CIRCLE('',#21,1.);\n", true).replace(
        "#12=ADVANCED_FACE('',(#13),#20,.T.);",
        "#12=ADVANCED_FACE('',(#13),#30,.T.);",
    );
    let e = error(&text);
    assert!(e.contains("#30 CIRCLE") && e.contains("surface"), "{e}");
    // An oriented face of an oriented face.
    let text = file(
        "#30=CIRCLE('',#21,1.);\n#40=ORIENTED_FACE('',*,#41,.F.);\n#41=ORIENTED_FACE('',*,#12,.F.);\n",
        true,
    )
    .replace("#11=CLOSED_SHELL('',(#12));", "#11=CLOSED_SHELL('',(#40));");
    let e = error(&text);
    assert!(e.contains("oriented face of an oriented face"), "{e}");
}

#[test]
fn absurd_numbers_are_refused() {
    // A radius beyond what a double holds once in millimetres of metres.
    let text = file("#30=CIRCLE('',#21,1.E308);\n", true)
        .replace("SI_UNIT(.MILLI.,.METRE.)", "SI_UNIT(.KILO.,.METRE.)");
    let e = error(&text);
    assert!(e.contains("too large") || e.contains("not finite"), "{e}");
    // A degree of a thousand, and multiplicities that claim a billion
    // knots: refused before anything is allocated.
    let e = error(&file(
        "#30=B_SPLINE_CURVE_WITH_KNOTS('',1000,(#18,#19),.UNSPECIFIED.,.F.,.F.,(2,2),(0.,1.),.UNSPECIFIED.);\n",
        true,
    ));
    assert!(e.contains("degree"), "{e}");
    let e = error(&file(
        "#30=B_SPLINE_CURVE_WITH_KNOTS('',1,(#18,#19),.UNSPECIFIED.,.F.,.F.,(1000000000,2),(0.,1.),.UNSPECIFIED.);\n",
        true,
    ));
    assert!(e.contains("multiplicities add up"), "{e}");
}

#[test]
fn a_mapped_item_of_the_wrong_kinds_is_named() {
    // A mapped item listed in the part's representation whose source is
    // a representation, not a representation map.
    let text = file(
        "#30=CIRCLE('',#21,1.);\n#50=MAPPED_ITEM('',#5,#21);\n",
        true,
    )
    .replace(
        "#5=ADVANCED_BREP_SHAPE_REPRESENTATION('part',(#10),#4);",
        "#5=ADVANCED_BREP_SHAPE_REPRESENTATION('part',(#10,#50),#4);",
    );
    let e = match parse(text.as_bytes()) {
        Ok(_) => panic!("read"),
        Err(e) => e.to_string(),
    };
    assert!(e.contains("REPRESENTATION_MAP"), "{e}");
}

/// The knots ISO 10303-42 implies for uniform, quasi-uniform and
/// piecewise Bézier B-splines.
#[test]
fn implied_knots_follow_the_types() {
    let text = file("#30=CIRCLE('',#21,1.);\n", true);
    let parsed = parse(text.as_bytes()).unwrap();
    let reader = geometry::Reader::new(&parsed, 1.0, 1.0);
    let knots = |form, count, degree| reader.implied_knots(1, form, count, degree);
    use geometry::Implicit::*;
    assert_eq!(
        knots(Uniform, 4, 2).unwrap(),
        [-2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0]
    );
    assert_eq!(
        knots(QuasiUniform, 5, 2).unwrap(),
        [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0]
    );
    assert_eq!(
        knots(Bezier, 7, 3).unwrap(),
        [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0]
    );
    assert!(
        knots(Bezier, 6, 3)
            .unwrap_err()
            .to_string()
            .contains("Bézier")
    );
    assert!(knots(QuasiUniform, 2, 2).is_err());
    // A polyline written as a quasi-uniform curve of degree one through
    // three points, read and evaluated.
    let text = file(
        "#30=QUASI_UNIFORM_CURVE('',1,(#18,#31,#32),.POLYLINE_FORM.,.F.,.F.);\n#31=CARTESIAN_POINT('',(1.,1.,0.));\n#32=CARTESIAN_POINT('',(0.,1.,0.));\n",
        true,
    );
    let parsed = parse(text.as_bytes()).unwrap();
    let reader = geometry::Reader::new(&parsed, 1.0, 1.0);
    let curve = reader.curve(30).unwrap();
    assert_eq!(curve.domain(), (0.0, 2.0));
    assert!(
        curve
            .eval(1.5)
            .distance(craft3d_math::Vec3::new(0.5, 1.0, 0.0))
            < 1e-15
    );
    // Bézier and uniform surfaces: the net as given, the knots implied.
    let net = "#31=CARTESIAN_POINT('',(0.,0.,0.));\n#32=CARTESIAN_POINT('',(0.,1.,0.));\n#33=CARTESIAN_POINT('',(1.,0.,0.));\n#34=CARTESIAN_POINT('',(1.,1.,1.));\n";
    for keyword in ["BEZIER_SURFACE", "UNIFORM_SURFACE", "QUASI_UNIFORM_SURFACE"] {
        let text = file(
            &format!(
                "#30=CIRCLE('',#21,1.);\n{net}#40={keyword}('',1,1,((#31,#32),(#33,#34)),.UNSPECIFIED.,.F.,.F.,.F.);\n"
            ),
            true,
        );
        let parsed = parse(text.as_bytes()).unwrap();
        let reader = geometry::Reader::new(&parsed, 1.0, 1.0);
        match reader.surface(40).unwrap() {
            geometry::Read::Done(craft3d_geom::Surface::Nurbs(n), false) => {
                assert_eq!(n.counts(), (2, 2), "{keyword}");
                let ((u0, u1), (v0, v1)) = n.domain();
                let corner = n.eval(u1, v1);
                assert!(corner.distance(craft3d_math::Vec3::new(1.0, 1.0, 1.0)) < 1e-15);
                assert!(n.eval(u0, v0).length() < 1e-15, "{keyword}");
            }
            other => panic!("{keyword}: {other:?}"),
        }
    }
}

/// Swept and offset surfaces that cannot be read are named; a swept
/// surface of itself ends.
#[test]
fn swept_and_offset_surfaces_that_are_not_read_are_named() {
    let text = file(
        "#30=CIRCLE('',#21,1.);\n#40=OFFSET_SURFACE('',#41,1.,.F.);\n#41=OFFSET_SURFACE('',#40,1.,.F.);\n#42=SURFACE_OF_REVOLUTION('',#43,#44);\n#43=LINE('',#18,#45);\n#44=AXIS1_PLACEMENT('',#19,$);\n#45=VECTOR('',#46,1.);\n#46=DIRECTION('',(0.,1.,0.));\n#47=OFFSET_SURFACE('',#20,-5.,.F.);\n#48=SPHERICAL_SURFACE('',#21,1.);\n#49=OFFSET_SURFACE('',#48,-1.,.F.);\n",
        true,
    );
    let parsed = parse(text.as_bytes()).unwrap();
    let reader = geometry::Reader::new(&parsed, 1.0, 1.0);
    let e = reader.surface(40).unwrap_err().to_string();
    assert!(e.contains("more than"), "{e}");
    // A line that does not meet the axis: a hyperboloid.
    let e = reader.surface(42).unwrap_err().to_string();
    assert!(e.contains("hyperboloid"), "{e}");
    // A plane offset: read; a sphere offset through its centre: refused.
    assert!(reader.surface(47).is_ok());
    let e = reader.surface(49).unwrap_err().to_string();
    assert!(e.contains("centre"), "{e}");
    // A cone that opens towards -z (a negative semi-angle): offset
    // outward, its radius grows (the side is taken on its own nappe).
    let text = file(
        "#30=CIRCLE('',#21,1.);\n#50=CONICAL_SURFACE('',#21,2.,-0.5);\n#51=OFFSET_SURFACE('',#50,0.5,.F.);\n",
        true,
    );
    let parsed = parse(text.as_bytes()).unwrap();
    let reader = geometry::Reader::new(&parsed, 1.0, 1.0);
    match reader.surface(51).unwrap() {
        geometry::Read::Done(craft3d_geom::Surface::Cone(c), false) => {
            assert!(
                (c.radius() - (2.0 + 0.5 / 0.5f64.pcos())).abs() < 1e-12,
                "{c:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn broken_text_is_an_error() {
    let text = file("#30=CIRCLE('',#21,1.);\n", true);
    for cut in [0, 10, text.len() / 2, text.len() - 5] {
        assert!(parse(&text.as_bytes()[..cut]).is_err());
    }
    // A reference to an instance that does not exist.
    let missing = text.replace("#14=EDGE_LOOP('',(#15));", "#14=EDGE_LOOP('',(#99));");
    assert!(parse(missing.as_bytes()).is_err());
    // No such item.
    let parsed = parse(text.as_bytes()).unwrap();
    assert_eq!(parsed.items().len(), 1);
    assert_eq!(parsed.items()[0].name, "solid");
    assert!(parsed.build(7, &options()).is_err());
    let bad_unit = Options {
        metres_per_unit: f64::NAN,
        ..options()
    };
    assert!(parsed.build(0, &bad_unit).is_err());
}

#[test]
fn the_shared_parse_is_reused_by_key() {
    let text = file("#30=CIRCLE('',#21,1.);\n", true);
    let a = parse_shared(text.as_bytes(), "k1").unwrap();
    let b = parse_shared(b"not even step", "k1").unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert!(parse_shared(b"not even step", "k2").is_err());
    forget_shared();
}

/// Review of 2026-10-21: one style assignment of n entries shared by n
/// styled items was followed n² times inside every parse (6 s at n =
/// 8000; a 128 MiB file of that shape for days). Each style entity is
/// now read once: n = 40 000 parses within seconds, the body read.
#[test]
fn a_style_shared_by_many_styled_items_is_read_once() {
    let n = 40_000;
    let mut extra = String::from("#30=CIRCLE('',#21,1.);\n");
    let entries = vec!["#10"; n].join(",");
    extra.push_str(&format!(
        "#60=PRESENTATION_STYLE_ASSIGNMENT(({entries}));\n"
    ));
    for k in 0..n {
        extra.push_str(&format!("#{}=STYLED_ITEM('',(#60),#10);\n", 100 + k));
    }
    let text = file(&extra, true);
    let started = std::time::Instant::now();
    let parsed = parse(text.as_bytes()).unwrap();
    assert!(
        started.elapsed().as_secs_f64() < 5.0,
        "{:?}",
        started.elapsed()
    );
    assert!(parsed.colors.is_empty());
}
