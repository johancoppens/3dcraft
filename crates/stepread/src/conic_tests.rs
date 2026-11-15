//! Hyperbolas, parabolas, trimmed and composite curves and the swept and
//! offset surfaces they make, read from hand-written files (ISO 10303-42
//! entities as the standard defines them). Whole bodies cut from cones,
//! held to closed forms and to the same bodies built by the kernel, are
//! in `crates/io/tests/step_conics.rs`.

use super::*;
use crate::conic::Conic;
use crate::geometry::{CurveRead, Read, Reader};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Portable, Vec3};

const HEAD: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n";
const TAIL: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

/// A file of `extra` and the shared entities: `#18` the point (1, 0, 0),
/// `#19` the origin, `#21` the world placement, `#22` a placement at the
/// origin whose x is −z and z is +x (a plane through the world's z axis),
/// `#23` the direction (0, 0, 1), `#24` (1, 0, 0), `#25` a vector of
/// length 1 along z, `#26` the z axis as an `AXIS1_PLACEMENT`.
fn file(extra: &str) -> String {
    format!(
        "{HEAD}#1=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
         #2=( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );\n\
         #4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2)) REPRESENTATION_CONTEXT('','') );\n\
         #18=CARTESIAN_POINT('',(1.,0.,0.));\n\
         #19=CARTESIAN_POINT('',(0.,0.,0.));\n\
         #21=AXIS2_PLACEMENT_3D('',#19,$,$);\n\
         #22=AXIS2_PLACEMENT_3D('',#19,#24,#27);\n\
         #23=DIRECTION('',(0.,0.,1.));\n\
         #24=DIRECTION('',(1.,0.,0.));\n\
         #25=VECTOR('',#23,1.);\n\
         #26=AXIS1_PLACEMENT('',#19,#23);\n\
         #27=DIRECTION('',(0.,0.,-1.));\n\
         {extra}{TAIL}"
    )
}

fn parsed(extra: &str) -> Parsed {
    parse(file(extra).as_bytes()).unwrap()
}

fn conic_of(read: CurveRead) -> (Conic, Option<(f64, f64)>) {
    match read {
        CurveRead::Conic(c, range) => (c, range),
        other => panic!("not a conic: {other:?}"),
    }
}

/// `HYPERBOLA` and `PARABOLA`, directly and through surface and seam
/// curves, read as the standard's curves, lengths in the caller's unit.
#[test]
fn hyperbolas_and_parabolas_read_as_the_standards_curves() {
    let p = parsed(
        "#30=HYPERBOLA('',#22,2.,0.5);\n#31=PARABOLA('',#22,-0.75);\n#32=SURFACE_CURVE('',#30,(),.CURVE_3D.);\n#33=SEAM_CURVE('',#31,(),.PCURVE_S1.);\n",
    );
    // The file's millimetres read into a unit of half a millimetre.
    let reader = Reader::new(&p, 2.0, 1.0, 1e-9);
    for (id, hyperbola) in [(30, true), (31, false), (32, true), (33, false)] {
        let (conic, range) = conic_of(reader.curve(id).unwrap());
        assert_eq!(range, None);
        for u in [-1.5, 0.0, 0.4, 2.0] {
            // `#22`: z = (1, 0, 0), x = (0, 0, −1), so y = z × x = (0, 1, 0).
            let expected = if hyperbola {
                Vec3::new(0.0, 0.5 * u.psinh(), -2.0 * u.pcosh()) * 2.0
            } else {
                Vec3::new(0.0, 2.0 * -0.75 * u, -(-0.75 * u * u)) * 2.0
            };
            assert!(conic.point(u).distance(expected) < 1e-14, "#{id} at {u}");
        }
    }
}

/// `TRIMMED_CURVE` of a hyperbola or parabola: by parameter, by point, by
/// both (the file's preference taken), either sense; refused when the
/// trims are the same, run against the sense, lie off the curve, are not
/// finite, or trim a trimmed curve.
#[test]
fn trimmed_conics_keep_their_trims_in_either_sense() {
    let base = "#30=HYPERBOLA('',#21,2.,1.);\n#31=CARTESIAN_POINT('',(3.0861612696304874,1.1752011936438014,0.));\n#32=CARTESIAN_POINT('',(2.,0.,0.));\n#33=PARABOLA('',#21,0.5);\n#34=CARTESIAN_POINT('',(2.,2.,0.));\n";
    let p = parsed(&format!(
        "{base}\
         #40=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(-0.5)),(PARAMETER_VALUE(1.)),.T.,.PARAMETER.);\n\
         #41=TRIMMED_CURVE('',#30,(#32),(#31),.T.,.CARTESIAN.);\n\
         #42=TRIMMED_CURVE('',#30,(#31,PARAMETER_VALUE(1.)),(PARAMETER_VALUE(-0.5),#32),.F.,.UNSPECIFIED.);\n\
         #43=TRIMMED_CURVE('',#33,(PARAMETER_VALUE(-1.)),(#34),.T.,.UNSPECIFIED.);\n\
         #44=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(1.)),(PARAMETER_VALUE(1.)),.T.,.PARAMETER.);\n\
         #45=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(1.)),(PARAMETER_VALUE(-1.)),.T.,.PARAMETER.);\n\
         #46=TRIMMED_CURVE('',#30,(#18),(#31),.T.,.CARTESIAN.);\n\
         #47=TRIMMED_CURVE('',#40,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(0.5)),.T.,.PARAMETER.);\n\
         #48=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(-0.5)),(PARAMETER_VALUE(1.E300)),.T.,.PARAMETER.);\n\
         #49=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(-0.5),PARAMETER_VALUE(1.)),(#31),.T.,.PARAMETER.);\n\
         #50=TRIMMED_CURVE('',#30,(),(#31),.T.,.PARAMETER.);\n\
         #51=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(2.)),.T.,.PARAMETER.);\n"
    ));
    let reader = Reader::new(&p, 1.0, 1.0, 1e-9);
    // By parameter and by point: the arc from −0.5 (or the vertex) to 1.
    let (c, range) = conic_of(reader.curve(40).unwrap());
    assert_eq!(range, Some((-0.5, 1.0)));
    assert!(
        c.point(1.0)
            .distance(Vec3::new(2.0 * 1f64.pcosh(), 1f64.psinh(), 0.0))
            < 1e-15
    );
    let (c, range) = conic_of(reader.curve(41).unwrap());
    let (u0, u1) = range.unwrap();
    assert!(u0.abs() < 1e-15 && (u1 - 1.0).abs() < 1e-14, "{range:?}");
    assert!(
        c.point(u1)
            .distance(Vec3::new(3.0861612696304874, 1.1752011936438014, 0.0))
            < 1e-14
    );
    // Against the hyperbola, the points taken (no preference, both given):
    // from (cosh 1, sinh 1) down to the vertex, the reversed conic's
    // parameter running from −1 to 0.
    let (c, range) = conic_of(reader.curve(42).unwrap());
    let (u0, u1) = range.unwrap();
    assert!((u0 + 1.0).abs() < 1e-14 && u1.abs() < 1e-15, "{range:?}");
    assert!(
        c.point(u0)
            .distance(Vec3::new(3.0861612696304874, 1.1752011936438014, 0.0))
            < 1e-14
    );
    assert!(c.point(u1).distance(Vec3::new(2.0, 0.0, 0.0)) < 1e-14);
    assert!(c.derivatives(u0).1.y < 0.0, "runs down the branch");
    // A parabola from −1 to the point (2, 2): u = y / 2F = 2.
    let (c, range) = conic_of(reader.curve(43).unwrap());
    assert_eq!(range, Some((-1.0, 2.0)));
    assert!(c.point(2.0).distance(Vec3::new(2.0, 2.0, 0.0)) < 1e-15);
    let refused = |id: u64, what: &str| {
        let e = reader.curve(id).unwrap_err().to_string();
        assert!(
            e.contains(&format!("#{id} TRIMMED_CURVE")) && e.contains(what),
            "#{id}: {e}"
        );
    };
    refused(44, "the same");
    refused(45, "against its sense_agreement");
    refused(46, "off its hyperbola");
    refused(47, "trimmed curve of a trimmed curve");
    refused(49, "other than one point and one parameter value");
    refused(50, "trim_1 is not a set");
    // A trim far out: its arc cannot be made, an error when it is.
    assert!(matches!(reader.curve(48), Ok(CurveRead::Conic(_, Some(_)))));
    let (c, range) = conic_of(reader.curve(48).unwrap());
    let (u0, u1) = range.unwrap();
    assert!(c.arc(u0, u1).is_err());
    // Two units: two pieces, five control points.
    let (c, range) = conic_of(reader.curve(51).unwrap());
    let (u0, u1) = range.unwrap();
    assert_eq!(c.arc(u0, u1).unwrap().control_points().len(), 5);
}

/// A trimmed curve of another kind is its basis, turned round for `.F.`;
/// its trims are read, so a malformed one is refused.
#[test]
fn a_trimmed_line_or_circle_is_its_basis_in_its_sense() {
    let p = parsed(
        "#30=LINE('',#19,#25);\n#31=CIRCLE('',#21,2.);\n\
         #40=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(3.)),.T.,.PARAMETER.);\n\
         #41=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(3.)),(PARAMETER_VALUE(0.)),.F.,.PARAMETER.);\n\
         #42=TRIMMED_CURVE('',#31,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(1.)),.F.,.PARAMETER.);\n\
         #43=TRIMMED_CURVE('',#31,(PARAMETER_VALUE(0.)),('a'),.T.,.PARAMETER.);\n",
    );
    let reader = Reader::new(&p, 1.0, 1.0, 1e-9);
    let curve = |id| match reader.curve(id).unwrap() {
        CurveRead::Curve(c) => c,
        other => panic!("{other:?}"),
    };
    assert_eq!(curve(40).eval(2.0), Vec3::new(0.0, 0.0, 2.0));
    assert_eq!(curve(41).eval(2.0), Vec3::new(0.0, 0.0, -2.0));
    let Curve3::Circle(c) = curve(42) else {
        panic!("not a circle")
    };
    assert_eq!(c.frame().z(), Vec3::new(0.0, 0.0, -1.0));
    let e = reader.curve(43).unwrap_err().to_string();
    assert!(
        e.contains("#43 TRIMMED_CURVE") && e.contains("trim_2"),
        "{e}"
    );
}

/// `COMPOSITE_CURVE`: a trimmed line, a trimmed circle (by angle in
/// degrees), a trimmed hyperbola and a B-spline, one segment turned
/// round, joined into one exact B-spline through each segment's points;
/// a gap between segments and an unbounded segment are refused.
#[test]
fn composite_curves_are_joined_into_one_exact_spline() {
    // A line from (−2, −1) to (0, −1), a quarter circle about the origin
    // from (0, −1) to (1, 0), then up the hyperbola x² − y² = 1 from its
    // vertex to u = 1, then a straight B-spline from there, written from
    // its end so that its segment runs against it.
    let h = (1f64.pcosh(), 1f64.psinh());
    let text = format!(
        "#30=CARTESIAN_POINT('',(-2.,-1.,0.));\n#31=DIRECTION('',(1.,0.,0.));\n#32=VECTOR('',#31,2.);\n#33=LINE('',#30,#32);\n\
         #34=TRIMMED_CURVE('',#33,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(1.)),.T.,.PARAMETER.);\n\
         #35=CIRCLE('',#21,1.);\n\
         #36=TRIMMED_CURVE('',#35,(PARAMETER_VALUE(270.)),(PARAMETER_VALUE(360.)),.T.,.PARAMETER.);\n\
         #37=HYPERBOLA('',#21,1.,1.);\n\
         #38=TRIMMED_CURVE('',#37,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(1.)),.T.,.PARAMETER.);\n\
         #39=CARTESIAN_POINT('',({},{},0.));\n#40=CARTESIAN_POINT('',(3.,3.,0.));\n\
         #41=B_SPLINE_CURVE_WITH_KNOTS('',1,(#40,#39),.UNSPECIFIED.,.F.,.F.,(2,2),(0.,1.),.UNSPECIFIED.);\n\
         #50=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#34);\n#51=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#36);\n#52=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#38);\n#53=COMPOSITE_CURVE_SEGMENT(.DISCONTINUOUS.,.F.,#41);\n\
         #60=COMPOSITE_CURVE('',(#50,#51,#52,#53),.F.);\n\
         #61=COMPOSITE_CURVE('',(#50,#52),.F.);\n\
         #62=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#33);\n#63=COMPOSITE_CURVE('',(#62),.F.);\n\
         #64=COMPOSITE_CURVE('',(),.F.);\n",
        h.0, h.1
    );
    let p = parsed(&text);
    // Degrees: the circle's trims are in degrees in this reader.
    let reader = Reader::new(&p, 1.0, std::f64::consts::PI / 180.0, 1e-9);
    let CurveRead::Curve(Curve3::Nurbs(joined)) = reader.curve(60).unwrap() else {
        panic!("not one spline")
    };
    assert_eq!(joined.degree(), 2);
    assert_eq!(joined.domain(), (0.0, 4.0));
    let at = |t: f64| joined.eval(t);
    assert!(at(0.0).distance(Vec3::new(-2.0, -1.0, 0.0)) < 1e-15);
    assert!(at(1.0).distance(Vec3::new(0.0, -1.0, 0.0)) < 1e-15);
    assert!(at(2.0).distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-15);
    assert!(at(3.0).distance(Vec3::new(h.0, h.1, 0.0)) < 1e-14);
    assert!(at(4.0).distance(Vec3::new(3.0, 3.0, 0.0)) < 1e-15);
    for k in 0..=40 {
        let s = f64::from(k) / 40.0;
        assert!((at(0.0 + s).y + 1.0).abs() < 1e-15, "the line");
        assert!((at(1.0 + s).length() - 1.0).abs() < 1e-14, "the circle");
        let q = at(2.0 + s);
        assert!((q.x * q.x - q.y * q.y - 1.0).abs() < 1e-13, "the hyperbola");
    }
    // A gap between the line's end and the hyperbola's vertex: named by
    // the segment that does not start where the one before ends.
    let e = reader.curve(61).unwrap_err().to_string();
    assert!(
        e.contains("#52 COMPOSITE_CURVE_SEGMENT")
            && e.contains("#61 COMPOSITE_CURVE")
            && e.contains("starts 1.4142"),
        "{e}"
    );
    let e = reader.curve(63).unwrap_err().to_string();
    assert!(e.contains("#33 LINE") && e.contains("must end"), "{e}");
    let e = reader.curve(64).unwrap_err().to_string();
    assert!(e.contains("#64 COMPOSITE_CURVE"), "{e}");
}

/// Swept hyperbolas and parabolas: untrimmed, bounded by their faces
/// later; trimmed, the exact B-spline surface at once; refused when the
/// sweep makes no surface or makes it twice, and as an offset's basis.
#[test]
fn swept_conics_are_read_or_refused_by_what_they_sweep() {
    let p = parsed(
        "#30=HYPERBOLA('',#21,1.,0.5);\n#31=PARABOLA('',#21,0.25);\n\
         #32=DIRECTION('',(1.,0.,0.));\n#33=VECTOR('',#32,2.);\n\
         #34=DIRECTION('',(0.,1.,0.));\n#35=AXIS1_PLACEMENT('',#19,#24);\n#36=AXIS1_PLACEMENT('',#19,#34);\n\
         #37=CARTESIAN_POINT('',(-3.,0.,0.));\n#38=AXIS1_PLACEMENT('',#37,#34);\n\
         #39=TRIMMED_CURVE('',#31,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(2.)),.T.,.PARAMETER.);\n\
         #40=SURFACE_OF_LINEAR_EXTRUSION('',#30,#25);\n\
         #41=SURFACE_OF_LINEAR_EXTRUSION('',#30,#33);\n\
         #42=SURFACE_OF_REVOLUTION('',#30,#36);\n\
         #43=SURFACE_OF_REVOLUTION('',#31,#35);\n\
         #44=SURFACE_OF_REVOLUTION('',#30,#35);\n\
         #45=SURFACE_OF_REVOLUTION('',#31,#38);\n\
         #46=SURFACE_OF_REVOLUTION('',#39,#35);\n\
         #47=OFFSET_SURFACE('',#40,0.1,.F.);\n\
         #48=SURFACE_OF_LINEAR_EXTRUSION('',#39,#25);\n",
    );
    let reader = Reader::new(&p, 1.0, 1.0, 1e-9);
    let read = |id| reader.surface(id);
    let refused = |id: u64, what: &str| {
        let e = read(id).unwrap_err().to_string();
        assert!(
            e.contains(&format!("#{id} ")) && e.contains(what),
            "#{id}: {e}"
        );
    };
    // Along z, out of the plane: bounded by the faces later.
    assert!(matches!(read(40), Ok(Read::Conic { .. })));
    // Along x, in its own plane: no surface.
    refused(41, "in its own plane");
    // A hyperbola turned about its imaginary axis: a hyperboloid of one
    // sheet, which it does not cross.
    assert!(matches!(read(42), Ok(Read::Conic { .. })));
    // A parabola about its own axis, a hyperbola about its real axis:
    // each crosses the axis, the surface swept twice.
    refused(43, "crosses");
    refused(44, "crosses");
    // A parabola about an axis in its plane behind its vertex, across its
    // own axis, which it never reaches.
    assert!(matches!(read(45), Ok(Read::Conic { .. })));
    // Trimmed to one side of its axis: the exact surface at once.
    assert!(matches!(read(46), Ok(Read::Done(Surface::Nurbs(_), false))));
    assert!(matches!(read(48), Ok(Read::Extrusion { .. })));
    refused(47, "offset of a swept hyperbola");
}

/// The finding: an offset towards the axis of a surface of revolution of
/// a line that meets the axis (a cone, read with its frame at the apex,
/// radius 0) was refused as going "through the axis", whatever the
/// distance. It is the cone with its apex moved along the axis by
/// `d / sin α`; offsets that pass a cylinder's axis or a sphere's centre
/// are still refused, saying what reads instead.
#[test]
fn a_cone_offset_towards_its_axis_moves_its_apex() {
    // The line from the apex (0, 0, 4) through (3, 0, 0), turned about z:
    // a cone narrowing upwards, its normal (∂u × ∂v, the line running
    // down) pointing inwards; offset by 0.5 along it.
    let p = parsed(
        "#30=CARTESIAN_POINT('',(0.,0.,4.));\n#31=DIRECTION('',(0.6,0.,-0.8));\n#32=VECTOR('',#31,1.);\n#33=LINE('',#30,#32);\n\
         #34=SURFACE_OF_REVOLUTION('',#33,#26);\n#35=OFFSET_SURFACE('',#34,0.5,.F.);\n#36=OFFSET_SURFACE('',#34,-0.5,.F.);\n\
         #37=CYLINDRICAL_SURFACE('',#21,1.);\n#38=OFFSET_SURFACE('',#37,-1.5,.F.);\n",
    );
    let reader = Reader::new(&p, 1.0, 1.0, 1e-9);
    let alpha = 0.6f64.patan2(0.8);
    for (id, d) in [(35, 0.5), (36, -0.5)] {
        let Ok(Read::Done(Surface::Cone(cone), opposite)) = reader.surface(id) else {
            panic!("#{id} is not read as a cone: {:?}", reader.surface(id))
        };
        assert!(opposite, "the file's normal points inwards");
        // The apex moves down (inwards offset) or up by d / sin α.
        let apex = cone.frame().point(0.0, 0.0, cone.apex_v());
        let expected = Vec3::new(0.0, 0.0, 4.0 - d / alpha.psin());
        assert!(apex.distance(expected) < 1e-14, "#{id}: {apex:?}");
        assert!((cone.half_angle().abs() - alpha).abs() < 1e-15);
        // Every point of the offset cone is d from the basis's, inwards.
        for v in [0.5, 1.5, 3.0] {
            let q = Surface::Cone(cone)
                .eval(0.3, cone.apex_v() + v * cone.half_angle().ptan().signum());
            let r = (q.x * q.x + q.y * q.y).sqrt();
            let on_basis = r * 0.8 + q.z * 0.6 - 4.0 * 0.6;
            assert!((on_basis + d).abs() < 1e-13, "#{id}: {on_basis}");
        }
    }
    let e = reader.surface(38).unwrap_err().to_string();
    assert!(
        e.contains("#38 OFFSET_SURFACE") && e.contains("axis") && e.contains("reads"),
        "{e}"
    );
}

/// A trimmed curve's parameter is its basis's: a trim on a trimmed
/// circle in a file in degrees is in degrees (it was taken as radians,
/// review of 2026-11-15); a parameter trim on a trimmed curve that runs
/// against its basis is refused, saying a trim by point reads.
#[test]
fn trims_on_a_trimmed_curve_have_its_basis_parameter() {
    let p = parsed(
        "#30=CIRCLE('',#21,1.);\n\
         #31=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(180.)),.T.,.PARAMETER.);\n\
         #32=TRIMMED_CURVE('',#31,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(90.)),.T.,.PARAMETER.);\n\
         #33=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#32);\n#34=COMPOSITE_CURVE('',(#33),.F.);\n\
         #35=TRIMMED_CURVE('',#30,(PARAMETER_VALUE(180.)),(PARAMETER_VALUE(0.)),.F.,.PARAMETER.);\n\
         #36=TRIMMED_CURVE('',#35,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(90.)),.T.,.PARAMETER.);\n\
         #37=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#36);\n#38=COMPOSITE_CURVE('',(#37),.F.);\n",
    );
    let reader = Reader::new(&p, 1.0, std::f64::consts::PI / 180.0, 1e-9);
    let CurveRead::Curve(Curve3::Nurbs(quarter)) = reader.curve(34).unwrap() else {
        panic!("not one spline")
    };
    let (a, b) = quarter.domain();
    assert!(quarter.eval(a).distance(Vec3::X) < 1e-15);
    assert!(quarter.eval(b).distance(Vec3::Y) < 1e-15);
    let e = reader.curve(38).unwrap_err().to_string();
    assert!(
        e.contains("#36 TRIMMED_CURVE") && e.contains("runs against its own basis"),
        "{e}"
    );
}

/// A line turned about an axis it meets, running from a point of the
/// cone up towards its apex: the file's normal is measured between the
/// apex and the point (at fixed parameters of the line it was measured
/// past the apex at a small scale, and refused, review of 2026-11-15).
#[test]
fn a_line_towards_its_apex_turns_into_a_cone_at_every_scale() {
    for scale in [1e-3, 1.0, 1e3] {
        let n = |x: f64| format!("{:.16E}", x * scale);
        let text = format!(
            "#30=CARTESIAN_POINT('',({},0.,{}));\n#31=DIRECTION('',(-0.6,0.,0.8));\n#32=VECTOR('',#31,{});\n#33=LINE('',#30,#32);\n#34=SURFACE_OF_REVOLUTION('',#33,#26);\n",
            n(3.0),
            n(-1.0),
            n(1.0)
        );
        let p = parsed(&text);
        let reader = Reader::new(&p, 1.0, 1.0, 1e-9 * scale);
        match reader.surface(34) {
            // ∂u × ∂v = (0.8x, 0, 0.6x): out from the axis and up, the
            // cone's own normal (it opens downwards from its apex).
            Ok(Read::Done(Surface::Cone(cone), false)) => {
                let apex = cone.frame().point(0.0, 0.0, cone.apex_v());
                assert!(apex.distance(Vec3::new(0.0, 0.0, 3.0 * scale)) < 1e-12 * scale);
            }
            other => panic!("@ {scale}: {other:?}"),
        }
    }
}

/// Hostile numbers and references in conics: never a panic, an error
/// naming the entity.
#[test]
fn hostile_conics_are_errors_that_name_them() {
    let cases = [
        (
            "#30=HYPERBOLA('',#21,0.,1.);\n",
            "#30 HYPERBOLA",
            "positive",
        ),
        (
            "#30=HYPERBOLA('',#21,-1.,1.);\n",
            "#30 HYPERBOLA",
            "other branch",
        ),
        (
            "#30=HYPERBOLA('',#21,1.,-2.);\n",
            "#30 HYPERBOLA",
            "positive",
        ),
        ("#30=HYPERBOLA('',#21,1.E308,1.);\n", "#30 HYPERBOLA", ""),
        ("#30=HYPERBOLA('',#21,1.);\n", "#30 HYPERBOLA", "too few"),
        (
            "#30=HYPERBOLA('',#18,1.,1.);\n",
            "#18",
            "AXIS2_PLACEMENT_3D",
        ),
        (
            "#30=HYPERBOLA('',#21,'a',1.);\n",
            "#30 HYPERBOLA",
            "semi_axis",
        ),
        ("#30=PARABOLA('',#21,0.);\n", "#30 PARABOLA", "not zero"),
        ("#30=PARABOLA('',#21,$);\n", "#30 PARABOLA", "focal_dist"),
        ("#30=PARABOLA('',#30,1.);\n", "#30", ""),
    ];
    for (text, entity, what) in cases {
        let p = parsed(text);
        // A thousand: the huge semi-axis overflows once converted.
        let reader = Reader::new(&p, 1000.0, 1.0, 1e-9);
        let e = reader.curve(30).unwrap_err().to_string();
        assert!(e.contains(entity) && e.contains(what), "{text}: {e}");
    }
    // An arc of a conic read is never a panic, whatever its ends.
    let p = parsed("#30=HYPERBOLA('',#21,1.,1.);\n");
    let reader = Reader::new(&p, 1.0, 1.0, 1e-9);
    let (c, _) = conic_of(reader.curve(30).unwrap());
    for (a, b) in [
        (0.0, 0.0),
        (1.0, -1.0),
        (f64::NAN, 0.0),
        (0.0, 1e10),
        (-800.0, 0.0),
    ] {
        assert!(c.arc(a, b).is_err(), "{a} {b}");
    }
}

/// The control-point budget counts a hyperbola as the arc it may become.
#[test]
fn the_budget_counts_conic_arcs() {
    let mut text = String::from("#30=HYPERBOLA('',#21,1.,1.);\n");
    let n = 4_000_000 / (2 * crate::conic::MAX_PIECES + 1) + 10;
    for k in 0..n {
        let id = 100 + 2 * k;
        text.push_str(&format!(
            "#{id}=EDGE_CURVE('',#{0},#{0},#30,.T.);\n#{1}=VERTEX_POINT('',#18);\n",
            id + 1,
            id + 1
        ));
    }
    let list: Vec<String> = (0..n).map(|k| format!("#{}", 100 + 2 * k)).collect();
    text.push_str(&format!(
        "#10=MANIFOLD_SOLID_BREP('s',#11);\n#11=CLOSED_SHELL('',(#12));\n#12=ADVANCED_FACE('',(#13),#20,.T.);\n#13=FACE_BOUND('',#14,.T.);\n#14=EDGE_LOOP('',({}));\n#20=PLANE('',#21);\n#5=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#10),#4);\n",
        list.join(",")
    ));
    let e = parse(file(&text).as_bytes()).unwrap_err().to_string();
    assert!(e.contains("control points"), "{e}");
}
