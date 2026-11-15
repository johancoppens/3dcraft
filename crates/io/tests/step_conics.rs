//! Hyperbolas and parabolas in STEP files (ISO 10303-42 `hyperbola`,
//! `parabola`), read by `craft3d-stepread`: the files are written here,
//! by hand from the standard, the way other modellers write the section
//! of a cone by a plane (parallel to its axis: a hyperbola; to a ruling:
//! a parabola). Each body read is held to the closed form of the cut cone
//! (volume and area), to `topo::check`, to its mesh, to the same body the
//! kernel makes (a cone cut by a block) and to itself exported and read
//! again; at three scales, two frames, three apex angles and two offsets,
//! with the conic written in every form an edge may take (plain, a
//! surface curve, trimmed by parameter or by point, against its sense, in
//! a composite curve, on a placement turned round). Then extruded and
//! turned conics, and a cone offset past its apex (`OFFSET_SURFACE` of a
//! surface of revolution of a line, which was refused).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_io::{StepBody, StepOptions, export_step};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_stepread::{Imported, Options, parse};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::Body;
use std::f64::consts::PI;

fn tolerance(scale: f64) -> Tolerance {
    Tolerance::new(1e-6 * scale.min(1.0), 1e-9).unwrap()
}

fn options(scale: f64) -> Options {
    Options {
        metres_per_unit: 1e-3,
        tolerance: tolerance(scale),
        loosest: 1000.0 * tolerance(scale).linear,
    }
}

fn frames(scale: f64) -> Vec<(&'static str, Frame)> {
    vec![
        ("world", Frame::WORLD),
        (
            "tilted",
            Frame::new(
                Vec3::new(1.0, -2.0, 0.5) * scale,
                Vec3::new(0.3, -0.4, 1.0),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap(),
        ),
    ]
}

/// A STEP file being written: entities in millimetres, the shape given
/// in local coordinates, placed by `frame` and multiplied by `scale`.
struct Step {
    text: String,
    next: u64,
    frame: Frame,
    scale: f64,
}

fn real(x: f64) -> String {
    format!("{x:.16E}")
}

impl Step {
    fn new(frame: Frame, scale: f64) -> Self {
        let mut step = Step {
            text: String::new(),
            next: 10,
            frame,
            scale,
        };
        step.text.push_str(&format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n\
             #1=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
             #2=( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );\n\
             #3=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({}),#1,'distance_accuracy_value','');\n\
             #4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#3)) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2)) REPRESENTATION_CONTEXT('','') );\n",
            real(tolerance(scale).linear)
        ));
        step
    }

    fn add(&mut self, entity: &str) -> u64 {
        let id = self.next;
        self.next += 1;
        self.text.push_str(&format!("#{id}={entity};\n"));
        id
    }

    fn world(&self, local: Vec3) -> Vec3 {
        let p = local * self.scale;
        self.frame.point(p.x, p.y, p.z)
    }

    fn point(&mut self, local: Vec3) -> u64 {
        let p = self.world(local);
        self.add(&format!(
            "CARTESIAN_POINT('',({},{},{}))",
            real(p.x),
            real(p.y),
            real(p.z)
        ))
    }

    fn direction(&mut self, local: Vec3) -> u64 {
        let d = self.frame.vector(local.x, local.y, local.z);
        self.add(&format!(
            "DIRECTION('',({},{},{}))",
            real(d.x),
            real(d.y),
            real(d.z)
        ))
    }

    fn placement(&mut self, origin: Vec3, z: Vec3, x: Vec3) -> u64 {
        let (o, z, x) = (self.point(origin), self.direction(z), self.direction(x));
        self.add(&format!("AXIS2_PLACEMENT_3D('',#{o},#{z},#{x})"))
    }

    fn vertex(&mut self, local: Vec3) -> (u64, u64) {
        let p = self.point(local);
        (self.add(&format!("VERTEX_POINT('',#{p})")), p)
    }

    /// A line through `at` along the unit `along` (its parameter a length
    /// in local units).
    fn line(&mut self, at: Vec3, along: Vec3) -> u64 {
        let (p, d) = (self.point(at), self.direction(along));
        let v = self.add(&format!("VECTOR('',#{d},{})", real(self.scale)));
        self.add(&format!("LINE('',#{p},#{v})"))
    }

    fn length(&self, x: f64) -> String {
        real(x * self.scale)
    }

    fn edge(&mut self, start: u64, end: u64, curve: u64, same: bool) -> u64 {
        self.add(&format!(
            "EDGE_CURVE('',#{start},#{end},#{curve},.{}.)",
            if same { "T" } else { "F" }
        ))
    }

    /// A face of `surface` with one loop of `(edge, forward)`.
    fn face(&mut self, surface: u64, same: bool, edges: &[(u64, bool)]) -> u64 {
        let oriented: Vec<String> = edges
            .iter()
            .map(|&(e, forward)| {
                let id = self.add(&format!(
                    "ORIENTED_EDGE('',*,*,#{e},.{}.)",
                    if forward { "T" } else { "F" }
                ));
                format!("#{id}")
            })
            .collect();
        let lp = self.add(&format!("EDGE_LOOP('',({}))", oriented.join(",")));
        let bound = self.add(&format!("FACE_OUTER_BOUND('',#{lp},.T.)"));
        self.add(&format!(
            "ADVANCED_FACE('',(#{bound}),#{surface},.{}.)",
            if same { "T" } else { "F" }
        ))
    }

    fn solid(mut self, faces: &[u64]) -> String {
        let list: Vec<String> = faces.iter().map(|f| format!("#{f}")).collect();
        let shell = self.add(&format!("CLOSED_SHELL('',({}))", list.join(",")));
        let solid = self.add(&format!("MANIFOLD_SOLID_BREP('cut',#{shell})"));
        self.add(&format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{solid}),#4)"
        ));
        self.text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        self.text
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cut {
    /// Parallel to the axis at `x = c`; the piece `x ≥ c`, apart from
    /// the apex.
    Hyperbola,
    /// Parallel to the ruling through `(R, 0, 0)`, through `(c, 0, 0)`:
    /// the piece `x + kz ≤ c` (`k = R / H`), apart from the apex.
    Parabola,
}

/// How the conic edge is written.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    Plain,
    SurfaceCurve,
    TrimmedByParameter,
    TrimmedByPoint,
    /// A trimmed curve running from the edge's end to its start, the edge
    /// against it.
    TrimmedAgainst,
    /// Two trimmed halves joined at the conic's vertex.
    Composite,
    /// The conic on its placement seen from the other side (`y`, `z`
    /// turned: the parameter runs the other way), the edge against it.
    Turned,
    /// A hyperbola on the placement turned about its normal: the other
    /// branch, which the edge's vertices are not on. Refused.
    OtherBranch,
    /// The conic as the exact rational quadratic B-spline of `n` spans a
    /// writer may give instead (`B_SPLINE_CURVE_WITH_KNOTS`, rational),
    /// its middle control points lifted off the cutting plane by the
    /// second number (in local units; 0 for the exact curve).
    Spline(usize, f64),
}

const FORMS: [Form; 9] = [
    Form::Plain,
    Form::SurfaceCurve,
    Form::TrimmedByParameter,
    Form::TrimmedByPoint,
    Form::TrimmedAgainst,
    Form::Composite,
    Form::Turned,
    Form::Spline(1, 0.0),
    Form::Spline(5, 0.0),
];

/// The cone of base radius `r` on the plane `z = 0` and apex `(0, 0, h)`
/// cut as `cut` says at `c`, written with its conic edge as `form`.
fn cut_cone(cut: Cut, r: f64, h: f64, c: f64, frame: Frame, scale: f64, form: Form) -> String {
    let mut s = Step::new(frame, scale);
    let k = r / h;
    let y0 = (r * r - c * c).sqrt();
    let (v1, p1) = s.vertex(Vec3::new(c, -y0, 0.0));
    let (v2, p2) = s.vertex(Vec3::new(c, y0, 0.0));
    // The conic: its placement (origin, z, x), its entity, and the
    // parameters of the edge's ends.
    let (origin, normal, axis, entity, u0) = match cut {
        Cut::Hyperbola => {
            let (a, b) = (c / k, c);
            (
                Vec3::new(c, 0.0, h),
                Vec3::X,
                Vec3::new(0.0, 0.0, -1.0),
                format!("HYPERBOLA('',#PLACE,{},{})", s.length(a), s.length(b)),
                (r / c).pacosh(),
            )
        }
        Cut::Parabola => {
            let root = (1.0 + k * k).sqrt();
            let focal = (r - c) * (k / root) / 2.0;
            (
                Vec3::new((c - r) / 2.0, 0.0, (r + c) / (2.0 * k)),
                Vec3::new(1.0, 0.0, k) * (1.0 / root),
                Vec3::new(k, 0.0, -1.0) * (1.0 / root),
                format!("PARABOLA('',#PLACE,{})", s.length(focal)),
                y0 / (2.0 * focal),
            )
        }
    };
    let (normal, axis) = match form {
        Form::Turned => (-normal, axis),
        // Turned half round about its y: x the other way, y kept.
        Form::OtherBranch => (-normal, -axis),
        _ => (normal, axis),
    };
    let place = s.placement(origin, normal, axis);
    let conic = s.add(&entity.replace("PLACE", &place.to_string()));
    let parameter = |u: f64| format!("PARAMETER_VALUE({})", real(u));
    let (geometry, same) = match form {
        Form::Plain | Form::OtherBranch => (conic, true),
        Form::Spline(spans, lift) => {
            // Equal spans of the standard's parameter, each one rational
            // quadratic piece (in local coordinates): a parabola's middle
            // point its polar form, a hyperbola's where the end tangents
            // meet, with weight cosh of the half width.
            let local = Frame::new(origin, normal, axis).unwrap();
            let piece = |lo: f64, hi: f64| -> (Vec3, f64) {
                match cut {
                    Cut::Hyperbola => {
                        let (a, b) = (c / k, c);
                        let (m, d) = (0.5 * (lo + hi), 0.5 * (hi - lo));
                        let w = d.pcosh();
                        (local.point(a * m.pcosh() / w, b * m.psinh() / w, 0.0), w)
                    }
                    Cut::Parabola => {
                        let focal = (r - c) * (k / (1.0 + k * k).sqrt()) / 2.0;
                        (local.point(focal * lo * hi, focal * (lo + hi), 0.0), 1.0)
                    }
                }
            };
            let on = |u: f64| -> Vec3 {
                match cut {
                    Cut::Hyperbola => local.point(c / k * u.pcosh(), c * u.psinh(), 0.0),
                    Cut::Parabola => {
                        let focal = (r - c) * (k / (1.0 + k * k).sqrt()) / 2.0;
                        local.point(focal * u * u, 2.0 * focal * u, 0.0)
                    }
                }
            };
            let mut points = vec![on(-u0)];
            let mut weights = vec![1.0];
            let step = 2.0 * u0 / spans as f64;
            for k in 0..spans {
                let lo = -u0 + step * k as f64;
                let hi = if k + 1 == spans { u0 } else { lo + step };
                let (middle, w) = piece(lo, hi);
                points.extend([middle + normal * lift, on(hi)]);
                weights.extend([w, 1.0]);
            }
            let ids: Vec<String> = points.iter().map(|&p| format!("#{}", s.point(p))).collect();
            let mut mults = vec!["3".to_string()];
            mults.extend((1..spans).map(|_| "2".to_string()));
            mults.push("3".to_string());
            let knots: Vec<String> = (0..=spans).map(|k| real(k as f64)).collect();
            let w: Vec<String> = weights.iter().map(|&w| real(w)).collect();
            (
                s.add(&format!(
                    "( BOUNDED_CURVE() B_SPLINE_CURVE(2,({}),.UNSPECIFIED.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS(({}),({}),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE(({})) REPRESENTATION_ITEM('') )",
                    ids.join(","),
                    mults.join(","),
                    knots.join(","),
                    w.join(",")
                )),
                true,
            )
        }
        Form::Turned => (conic, false),
        Form::SurfaceCurve => (
            s.add(&format!("SURFACE_CURVE('',#{conic},(),.CURVE_3D.)")),
            true,
        ),
        Form::TrimmedByParameter => (
            s.add(&format!(
                "TRIMMED_CURVE('',#{conic},({}),({}),.T.,.PARAMETER.)",
                parameter(-u0),
                parameter(u0)
            )),
            true,
        ),
        Form::TrimmedByPoint => (
            s.add(&format!(
                "TRIMMED_CURVE('',#{conic},(#{p1}),(#{p2}),.T.,.CARTESIAN.)"
            )),
            true,
        ),
        Form::TrimmedAgainst => (
            s.add(&format!(
                "TRIMMED_CURVE('',#{conic},({},#{p2}),(#{p1},{}),.F.,.PARAMETER.)",
                parameter(u0),
                parameter(-u0)
            )),
            false,
        ),
        Form::Composite => {
            let low = s.add(&format!(
                "TRIMMED_CURVE('',#{conic},({}),({}),.T.,.PARAMETER.)",
                parameter(-u0),
                parameter(0.0)
            ));
            let high = s.add(&format!(
                "TRIMMED_CURVE('',#{conic},({}),({}),.T.,.PARAMETER.)",
                parameter(0.0),
                parameter(u0)
            ));
            let a = s.add(&format!("COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#{low})"));
            let b = s.add(&format!(
                "COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#{high})"
            ));
            (s.add(&format!("COMPOSITE_CURVE('',(#{a},#{b}),.F.)")), true)
        }
    };
    let conic_edge = s.edge(v1, v2, geometry, same);
    // The base circle, the line along y at x = c, and the surfaces.
    let centre = s.placement(Vec3::ZERO, Vec3::Z, Vec3::X);
    let circle = s.add(&format!("CIRCLE('',#{centre},{})", s.length(r)));
    let line = s.line(Vec3::new(c, -y0, 0.0), Vec3::Y);
    let line_edge = s.edge(v1, v2, line, true);
    let down = s.placement(Vec3::ZERO, -Vec3::Z, Vec3::X);
    let cone = s.add(&format!(
        "CONICAL_SURFACE('',#{down},{},{})",
        s.length(r),
        real(k.patan())
    ));
    let base = s.add(&format!("PLANE('',#{down})"));
    let faces = match cut {
        Cut::Hyperbola => {
            let arc = s.edge(v1, v2, circle, true);
            let at = s.placement(Vec3::new(c, 0.0, 0.0), -Vec3::X, Vec3::Y);
            let plane = s.add(&format!("PLANE('',#{at})"));
            vec![
                s.face(base, true, &[(line_edge, true), (arc, false)]),
                s.face(plane, true, &[(line_edge, false), (conic_edge, true)]),
                s.face(cone, true, &[(arc, true), (conic_edge, false)]),
            ]
        }
        Cut::Parabola => {
            let arc = s.edge(v2, v1, circle, true);
            let root = (1.0 + k * k).sqrt();
            let at = s.placement(
                Vec3::new(c, 0.0, 0.0),
                Vec3::new(1.0, 0.0, k) * (1.0 / root),
                Vec3::Y,
            );
            let plane = s.add(&format!("PLANE('',#{at})"));
            vec![
                s.face(base, true, &[(line_edge, false), (arc, false)]),
                s.face(plane, true, &[(line_edge, true), (conic_edge, false)]),
                s.face(cone, true, &[(arc, true), (conic_edge, true)]),
            ]
        }
    };
    s.solid(&faces)
}

/// The volume and area of the cut piece, in closed form (in local units).
fn closed_form(cut: Cut, r: f64, h: f64, c: f64) -> (f64, f64) {
    let k = r / h;
    let slant = (1.0 + k * k).sqrt() / k;
    match cut {
        Cut::Hyperbola => {
            let s = (r * r - c * c).sqrt();
            let l = ((r + s) / c).pln();
            let volume =
                (r.ppowi(3) * (c / r).pacos() - 2.0 * c * r * s + c.ppowi(3) * l) / (3.0 * k);
            let segment = r * r * (c / r).pacos() - c * s;
            let area = segment * (1.0 + slant) + (r * s - c * c * l) / k;
            (volume, area)
        }
        Cut::Parabola => {
            let e = r - c;
            let t = ((r + c) / e).sqrt();
            let i = (t * t + 1.0).ppowi(3) * t.patan() / 6.0 + t.ppowi(5) / 6.0
                - 4.0 * t.ppowi(3) / 9.0
                - t / 6.0;
            let volume = e.ppowi(3) * i / (2.0 * k);
            let base = r * r * (-c / r).pacos() + c * (r * r - c * c).sqrt();
            let theta = (1.0 - e / r).pacos();
            let cot = (theta / 2.0).pcos() / (theta / 2.0).psin();
            let projected = r * r * (PI - theta) - e * e / 2.0 * (cot + cot.ppowi(3) / 3.0);
            let y0 = (r * r - c * c).sqrt();
            let sagitta = (r + c) / (2.0 * k) * (1.0 + k * k).sqrt();
            let area = base + projected * slant + 4.0 / 3.0 * y0 * sagitta;
            (volume, area)
        }
    }
}

/// The same piece made by the kernel: a cone cut by a block.
fn native(cut: Cut, r: f64, h: f64, c: f64, frame: Frame, scale: f64) -> Body {
    let (r, h, c) = (r * scale, h * scale, c * scale);
    // The cone's one vertex, on its base circle at its frame's x, put in
    // the part cut away, so the piece has the file's two vertices.
    let turned = match cut {
        Cut::Hyperbola => Frame::new(frame.origin(), frame.z(), -frame.x()).unwrap(),
        Cut::Parabola => frame,
    };
    let cone = primitives::cone(&turned, r, h).unwrap();
    let big = 4.0 * (r + h);
    let k = r / h;
    let local = match cut {
        Cut::Hyperbola => Frame::new(Vec3::new(c, -big, -big), Vec3::Z, Vec3::X).unwrap(),
        Cut::Parabola => {
            // The block below the plane x + kz = c: its top face on it.
            let n = Vec3::new(1.0, 0.0, k).normalized().unwrap();
            let x = Vec3::Y.cross(n);
            let on = Vec3::new(c, 0.0, 0.0);
            Frame::new(on - x * big - Vec3::Y * big - n * big, n, x).unwrap()
        }
    };
    let placed = Frame::new(
        frame.point(local.origin().x, local.origin().y, local.origin().z),
        frame.vector(local.z().x, local.z().y, local.z().z),
        frame.vector(local.x().x, local.x().y, local.x().z),
    )
    .unwrap();
    let block = match cut {
        Cut::Hyperbola => primitives::block(&placed, big, 2.0 * big, 2.0 * big).unwrap(),
        Cut::Parabola => primitives::block(&placed, 2.0 * big, 2.0 * big, big).unwrap(),
    };
    boolean(&cone, &block, BooleanOp::Intersect, &tolerance(scale))
        .unwrap()
        .body
}

fn read(text: &str, scale: f64) -> Result<Imported, String> {
    let parsed = parse(text.as_bytes()).map_err(|e| e.to_string())?;
    parsed.build(0, &options(scale)).map_err(|e| e.to_string())
}

fn measures(body: &Body) -> (f64, f64) {
    let m = craft3d_topo::mass_properties(body).unwrap();
    (m.volume, m.area)
}

fn relative(a: f64, b: f64) -> f64 {
    (a - b).abs() / a.abs().max(b.abs())
}

/// Every check one imported piece must pass; the worst differences seen.
#[derive(Debug, Default)]
struct Worst {
    closed_form: f64,
    native: f64,
    round_trip: f64,
    mesh: f64,
}

fn hold(
    what: &str,
    imported: &Imported,
    expected: (f64, f64),
    twin: Option<&Body>,
    scale: f64,
    worst: &mut Worst,
) {
    let body = &imported.body;
    let r = &imported.report;
    assert_eq!(
        (body.faces.len(), body.edges.len(), body.vertices.len()),
        (3, 3, 2),
        "{what}"
    );
    assert_eq!(r.unproven_edges, 0, "{what}: {r:?}");
    assert_eq!(r.loose_edges + r.loose_vertices, 0, "{what}: {r:?}");
    assert_eq!(
        r.turned_shells + r.sewn_edges + r.merged_edges,
        0,
        "{what}: {r:?}"
    );
    let defects = craft3d_topo::check(body, &tolerance(scale));
    assert!(defects.is_empty(), "{what}: {defects:?}");
    let (volume, area) = measures(body);
    let (v, a) = (expected.0 * scale.ppowi(3), expected.1 * scale.ppowi(2));
    let d = relative(volume, v).max(relative(area, a));
    worst.closed_form = worst.closed_form.max(d);
    assert!(d < 1e-9, "{what}: volume {volume} / {v}, area {area} / {a}");
    if let Some(twin) = twin {
        let (tv, ta) = measures(twin);
        let d = relative(volume, tv).max(relative(area, ta));
        worst.native = worst.native.max(d);
        assert!(
            d < 1e-9,
            "{what}: the kernel's {tv} / {ta}, read {volume} / {area}"
        );
        assert_eq!(
            (twin.faces.len(), twin.edges.len(), twin.vertices.len()),
            (3, 3, 2),
            "{what}: the kernel's piece"
        );
    }
    // The mesh: closed, its volume within area × chord of the exact one.
    let chord = 1e-3 * scale;
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "{what}: the mesh is not closed");
    let off = (welded.volume() - v).abs();
    worst.mesh = worst.mesh.max(off / (a * chord));
    assert!(
        off <= a * chord,
        "{what}: mesh volume {} / {v}",
        welded.volume()
    );
    // Exported and read again: the same body.
    let (bytes, _) = export_step(
        &[StepBody::new("cut", body)],
        &StepOptions {
            tolerance: tolerance(scale),
            ..StepOptions::default()
        },
    )
    .unwrap();
    let again = parse(&bytes).unwrap().build(0, &options(scale)).unwrap();
    let (av, aa) = measures(&again.body);
    let d = relative(volume, av).max(relative(area, aa));
    worst.round_trip = worst.round_trip.max(d);
    assert!(d < 1e-12, "{what}: exported and read again {av} / {aa}");
    assert_eq!(again.report.unproven_edges, 0, "{what}: again");
}

/// The cut cones at three scales, two frames, three apex angles and two
/// offsets, against their closed forms and the kernel's own cut.
#[test]
fn cones_cut_parallel_to_their_axis_or_a_ruling_read_as_the_kernel_cuts_them() {
    let mut worst = Worst::default();
    let mut count = 0;
    for scale in [1e-3, 1.0, 1e6] {
        for (name, frame) in frames(scale) {
            // Base radius 2; heights for half-angles of about 63°, 34°
            // and 11°.
            for h in [1.0, 3.0, 10.0] {
                for c in [0.3, 1.4] {
                    for cut in [Cut::Hyperbola, Cut::Parabola] {
                        let what = format!("{cut:?} h {h} c {c} {name} @ {scale}");
                        let text = cut_cone(cut, 2.0, h, c, frame, scale, Form::Plain);
                        let imported = read(&text, scale).unwrap_or_else(|e| panic!("{what}: {e}"));
                        let twin = native(cut, 2.0, h, c, frame, scale);
                        hold(
                            &what,
                            &imported,
                            closed_form(cut, 2.0, h, c),
                            Some(&twin),
                            scale,
                            &mut worst,
                        );
                        count += 1;
                    }
                }
            }
        }
    }
    println!("{count} cut cones; worst relative difference {worst:?}");
}

/// Every form a conic edge may take reads as the same body; the other
/// branch is refused, naming what lies off it.
#[test]
fn every_form_of_the_conic_edge_reads_the_same_body() {
    let mut worst = Worst::default();
    for scale in [1e-3, 1.0, 1e6] {
        let frame = frames(scale)[1].1;
        for cut in [Cut::Hyperbola, Cut::Parabola] {
            let expected = closed_form(cut, 2.0, 3.0, 0.8);
            for form in FORMS {
                let what = format!("{cut:?} {form:?} @ {scale}");
                let text = cut_cone(cut, 2.0, 3.0, 0.8, frame, scale, form);
                let imported = read(&text, scale).unwrap_or_else(|e| panic!("{what}: {e}"));
                hold(&what, &imported, expected, None, scale, &mut worst);
            }
        }
        let text = cut_cone(
            Cut::Hyperbola,
            2.0,
            3.0,
            0.8,
            frame,
            scale,
            Form::OtherBranch,
        );
        let e = read(&text, scale).unwrap_err();
        assert!(
            e.contains("`cut`") && (e.contains("vertex #") || e.contains("edge #")),
            "{e}"
        );
    }
    println!("worst relative difference {worst:?}");
}

/// A trough under a parabola or a hyperbola: the curve extruded (as the
/// untrimmed conic, which the faces bound, and as a trimmed curve),
/// closed by a plane and two ends.
fn trough(hyperbola: bool, trimmed: bool, frame: Frame, scale: f64) -> (String, f64, f64) {
    let (focal, a, b, x0, h): (f64, f64, f64, f64, f64) = (0.4, 0.7, 0.5, 2.0, 1.5);
    let (u0, y0) = if hyperbola {
        let u0 = (x0 / a).pacosh();
        (u0, b * u0.psinh())
    } else {
        let u0 = (x0 / focal).sqrt();
        (u0, 2.0 * focal * u0)
    };
    let mut s = Step::new(frame, scale);
    let conic = |s: &mut Step, z: f64| {
        let place = s.placement(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X);
        if hyperbola {
            let (a, b) = (s.length(a), s.length(b));
            s.add(&format!("HYPERBOLA('',#{place},{a},{b})"))
        } else {
            let f = s.length(focal);
            s.add(&format!("PARABOLA('',#{place},{f})"))
        }
    };
    let (b1, _) = s.vertex(Vec3::new(x0, -y0, 0.0));
    let (b2, _) = s.vertex(Vec3::new(x0, y0, 0.0));
    let (t1, _) = s.vertex(Vec3::new(x0, -y0, h));
    let (t2, _) = s.vertex(Vec3::new(x0, y0, h));
    let bottom_curve = conic(&mut s, 0.0);
    let top_curve = conic(&mut s, h);
    let pb = s.edge(b1, b2, bottom_curve, true);
    let pt = s.edge(t1, t2, top_curve, true);
    let l1 = s.line(Vec3::new(x0, -y0, 0.0), Vec3::Z);
    let l2 = s.line(Vec3::new(x0, y0, 0.0), Vec3::Z);
    let lb = s.line(Vec3::new(x0, -y0, 0.0), Vec3::Y);
    let lt = s.line(Vec3::new(x0, -y0, h), Vec3::Y);
    let e1 = s.edge(b1, t1, l1, true);
    let e2 = s.edge(b2, t2, l2, true);
    let eb = s.edge(b1, b2, lb, true);
    let et = s.edge(t1, t2, lt, true);
    let swept = if trimmed {
        let p = |u: f64| format!("PARAMETER_VALUE({})", real(u));
        s.add(&format!(
            "TRIMMED_CURVE('',#{bottom_curve},({}),({}),.T.,.PARAMETER.)",
            p(-u0),
            p(u0)
        ))
    } else {
        bottom_curve
    };
    let up = s.direction(Vec3::Z);
    let along = s.add(&format!("VECTOR('',#{up},{})", real(scale)));
    let side = s.add(&format!(
        "SURFACE_OF_LINEAR_EXTRUSION('',#{swept},#{along})"
    ));
    let flat_at = s.placement(Vec3::new(x0, 0.0, 0.0), Vec3::X, Vec3::Y);
    let flat = s.add(&format!("PLANE('',#{flat_at})"));
    let floor_at = s.placement(Vec3::ZERO, -Vec3::Z, Vec3::X);
    let floor = s.add(&format!("PLANE('',#{floor_at})"));
    let lid_at = s.placement(Vec3::new(0.0, 0.0, h), Vec3::Z, Vec3::X);
    let lid = s.add(&format!("PLANE('',#{lid_at})"));
    let faces = vec![
        // The extrusion's normal C'(u) × V points into the trough.
        s.face(
            side,
            false,
            &[(pt, true), (e2, false), (pb, false), (e1, true)],
        ),
        s.face(
            flat,
            true,
            &[(eb, true), (e2, true), (et, false), (e1, false)],
        ),
        s.face(floor, true, &[(pb, true), (eb, false)]),
        s.face(lid, true, &[(et, true), (pt, false)]),
    ];
    let (section, length) = if hyperbola {
        // The arc length by Simpson's rule (no closed form).
        let n = 20_000;
        let speed = |t: f64| (a * a * t.psinh().ppowi(2) + b * b * t.pcosh().ppowi(2)).sqrt();
        let step = 2.0 * u0 / f64::from(n);
        let mut sum = speed(-u0) + speed(u0);
        for i in 1..n {
            let t = -u0 + step * f64::from(i);
            sum += speed(t) * if i % 2 == 1 { 4.0 } else { 2.0 };
        }
        (x0 * y0 - a * b * u0, sum * step / 3.0)
    } else {
        (
            4.0 / 3.0 * x0 * y0,
            y0 * (1.0 + u0 * u0).sqrt() + 2.0 * focal * u0.pasinh(),
        )
    };
    let volume = section * h;
    let area = 2.0 * section + 2.0 * y0 * h + length * h;
    (s.solid(&faces), volume, area)
}

#[test]
fn extruded_hyperbolas_and_parabolas_read_with_their_closed_forms() {
    for scale in [1e-3, 1.0, 1e6] {
        for (name, frame) in frames(scale) {
            for hyperbola in [false, true] {
                for trimmed in [false, true] {
                    let what = format!("hyperbola {hyperbola} trimmed {trimmed} {name} @ {scale}");
                    let (text, volume, area) = trough(hyperbola, trimmed, frame, scale);
                    let imported = read(&text, scale).unwrap_or_else(|e| panic!("{what}: {e}"));
                    let r = &imported.report;
                    assert_eq!(
                        r.unproven_edges + r.loose_edges + r.loose_vertices,
                        0,
                        "{what}: {r:?}"
                    );
                    let (v, a) = measures(&imported.body);
                    let (ev, ea) = (volume * scale.ppowi(3), area * scale.ppowi(2));
                    assert!(
                        relative(v, ev) < 1e-9 && relative(a, ea) < 1e-9,
                        "{what}: {v} / {ev}, {a} / {ea}"
                    );
                    assert!(craft3d_topo::check(&imported.body, &tolerance(scale)).is_empty());
                }
            }
        }
    }
}

/// A hyperboloid of one sheet (a hyperbola turned about its imaginary
/// axis, with a seam on the profile) between two discs, and a paraboloid
/// cup (a parabola trimmed to one side of its axis, turned about it)
/// closed by a disc; an untrimmed parabola turned about its own axis is
/// refused, saying why.
fn turned(hyperbola: bool, trimmed: bool, frame: Frame, scale: f64) -> (String, f64, f64) {
    let mut s = Step::new(frame, scale);
    let axis_at = s.point(Vec3::ZERO);
    let axis_dir = s.direction(Vec3::Z);
    let axis = s.add(&format!("AXIS1_PLACEMENT('',#{axis_at},#{axis_dir})"));
    let circle = |s: &mut Step, z: f64, r: f64| {
        let at = s.placement(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X);
        let r = s.length(r);
        s.add(&format!("CIRCLE('',#{at},{r})"))
    };
    let disc = |s: &mut Step, z: f64, up: bool| {
        let at = s.placement(
            Vec3::new(0.0, 0.0, z),
            if up { Vec3::Z } else { -Vec3::Z },
            Vec3::X,
        );
        s.add(&format!("PLANE('',#{at})"))
    };
    if hyperbola {
        let (a, b, u1, u2) = (1.2, 0.8, -1.1, 0.7);
        let r = |u: f64| a * u.pcosh();
        let z = |u: f64| b * u.psinh();
        // The profile in the xz plane: x = a cosh u, z = b sinh u.
        let place = s.placement(Vec3::ZERO, -Vec3::Y, Vec3::X);
        let (la, lb) = (s.length(a), s.length(b));
        let profile = s.add(&format!("HYPERBOLA('',#{place},{la},{lb})"));
        let (vb, _) = s.vertex(Vec3::new(r(u1), 0.0, z(u1)));
        let (vt, _) = s.vertex(Vec3::new(r(u2), 0.0, z(u2)));
        let cb = circle(&mut s, z(u1), r(u1));
        let ct = circle(&mut s, z(u2), r(u2));
        let bottom = s.edge(vb, vb, cb, true);
        let top = s.edge(vt, vt, ct, true);
        let seam_curve = s.add(&format!("SEAM_CURVE('',#{profile},(),.CURVE_3D.)"));
        let seam = s.edge(vb, vt, seam_curve, true);
        let side = s.add(&format!("SURFACE_OF_REVOLUTION('',#{profile},#{axis})"));
        let floor = disc(&mut s, z(u1), false);
        let lid = disc(&mut s, z(u2), true);
        let faces = vec![
            s.face(
                side,
                true,
                &[(bottom, true), (seam, true), (top, false), (seam, false)],
            ),
            s.face(floor, true, &[(bottom, false)]),
            s.face(lid, true, &[(top, true)]),
        ];
        let (z1, z2) = (z(u1), z(u2));
        let volume = PI * a * a * ((z2 - z1) + (z2.ppowi(3) - z1.ppowi(3)) / (3.0 * b * b));
        let n = 20_000;
        let ring = |t: f64| {
            2.0 * PI * r(t) * (a * a * t.psinh().ppowi(2) + b * b * t.pcosh().ppowi(2)).sqrt()
        };
        let step = (u2 - u1) / f64::from(n);
        let mut sum = ring(u1) + ring(u2);
        for i in 1..n {
            sum += ring(u1 + step * f64::from(i)) * if i % 2 == 1 { 4.0 } else { 2.0 };
        }
        let area = PI * (r(u1).ppowi(2) + r(u2).ppowi(2)) + sum * step / 3.0;
        (s.solid(&faces), volume, area)
    } else {
        let (focal, u1) = (0.3, 2.5);
        let (rho, h) = (2.0 * focal * u1, focal * u1 * u1);
        // Opening up along z, its y along x: radius 2Fu, height Fu².
        let place = s.placement(Vec3::ZERO, Vec3::Y, Vec3::Z);
        let f = s.length(focal);
        let parabola = s.add(&format!("PARABOLA('',#{place},{f})"));
        let profile = if trimmed {
            s.add(&format!(
                "TRIMMED_CURVE('',#{parabola},(PARAMETER_VALUE(0.)),(PARAMETER_VALUE({})),.T.,.PARAMETER.)",
                real(u1)
            ))
        } else {
            parabola
        };
        let (vr, _) = s.vertex(Vec3::new(rho, 0.0, h));
        let rim_curve = circle(&mut s, h, rho);
        let rim = s.edge(vr, vr, rim_curve, true);
        let side = s.add(&format!("SURFACE_OF_REVOLUTION('',#{profile},#{axis})"));
        let lid = disc(&mut s, h, true);
        let faces = vec![
            s.face(side, true, &[(rim, false)]),
            s.face(lid, true, &[(rim, true)]),
        ];
        let volume = PI * rho * rho * h / 2.0;
        let area = PI * rho * rho
            + PI * rho / (6.0 * h * h) * ((rho * rho + 4.0 * h * h).ppowf(1.5) - rho.ppowi(3));
        (s.solid(&faces), volume, area)
    }
}

#[test]
fn turned_hyperbolas_and_parabolas_read_with_their_closed_forms() {
    for scale in [1e-3, 1.0, 1e6] {
        for (name, frame) in frames(scale) {
            for hyperbola in [true, false] {
                let what = format!("hyperbola {hyperbola} {name} @ {scale}");
                let (text, volume, area) = turned(hyperbola, true, frame, scale);
                let imported = read(&text, scale).unwrap_or_else(|e| panic!("{what}: {e}"));
                let r = &imported.report;
                assert_eq!(
                    r.unproven_edges + r.loose_edges + r.loose_vertices,
                    0,
                    "{what}: {r:?}"
                );
                let (v, a) = measures(&imported.body);
                let (ev, ea) = (volume * scale.ppowi(3), area * scale.ppowi(2));
                assert!(
                    relative(v, ev) < 1e-9 && relative(a, ea) < 1e-9,
                    "{what}: {v} / {ev}, {a} / {ea}"
                );
                assert!(craft3d_topo::check(&imported.body, &tolerance(scale)).is_empty());
            }
            let (text, _, _) = turned(false, false, frame, scale);
            let e = read(&text, scale).unwrap_err();
            assert!(
                e.contains("SURFACE_OF_REVOLUTION") && e.contains("crosses"),
                "{e}"
            );
        }
    }
}

/// A cone whose side is an `OFFSET_SURFACE` of a surface of revolution of
/// a line that meets the axis, offset towards the axis: the finding,
/// refused before as an offset "through the axis". It reads as the cone
/// with its apex moved, with the cone's closed form.
fn offset_cone(frame: Frame, scale: f64, distance: f64) -> String {
    let (r, h) = (2.0, 3.0);
    let alpha = r.patan2(h);
    let mut s = Step::new(frame, scale);
    // The basis: the line from the apex moved up by d / sin α, down and
    // out along the cone's ruling, turned about z; its normal points in.
    let lift = distance / alpha.psin();
    let line = s.line(
        Vec3::new(0.0, 0.0, h + lift),
        Vec3::new(alpha.psin(), 0.0, -alpha.pcos()),
    );
    let axis_at = s.point(Vec3::ZERO);
    let axis_dir = s.direction(Vec3::Z);
    let axis = s.add(&format!("AXIS1_PLACEMENT('',#{axis_at},#{axis_dir})"));
    let basis = s.add(&format!("SURFACE_OF_REVOLUTION('',#{line},#{axis})"));
    let side = s.add(&format!(
        "OFFSET_SURFACE('',#{basis},{},.F.)",
        s.length(distance)
    ));
    let (v, _) = s.vertex(Vec3::new(r, 0.0, 0.0));
    let at = s.placement(Vec3::ZERO, Vec3::Z, Vec3::X);
    let circle = s.add(&format!("CIRCLE('',#{at},{})", s.length(r)));
    let rim = s.edge(v, v, circle, true);
    let down = s.placement(Vec3::ZERO, -Vec3::Z, Vec3::X);
    let floor = s.add(&format!("PLANE('',#{down})"));
    let faces = vec![
        s.face(side, false, &[(rim, true)]),
        s.face(floor, true, &[(rim, false)]),
    ];
    s.solid(&faces)
}

#[test]
fn a_cone_offset_past_its_apex_reads_as_the_cone_it_is() {
    let (r, h) = (2.0f64, 3.0f64);
    let volume = PI * r * r * h / 3.0;
    let area = PI * r * r + PI * r * (r * r + h * h).sqrt();
    for scale in [1e-3, 1.0, 1e6] {
        for (name, frame) in frames(scale) {
            for distance in [0.25, 1.0] {
                let what = format!("offset {distance} {name} @ {scale}");
                let imported = read(&offset_cone(frame, scale, distance), scale)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                let (v, a) = measures(&imported.body);
                let (ev, ea) = (volume * scale.ppowi(3), area * scale.ppowi(2));
                assert!(
                    relative(v, ev) < 1e-9 && relative(a, ea) < 1e-9,
                    "{what}: {v} / {ev}, {a} / {ea}"
                );
                let rp = &imported.report;
                assert_eq!(
                    rp.unproven_edges + rp.loose_edges + rp.loose_vertices,
                    0,
                    "{what}: {rp:?}"
                );
            }
        }
    }
}

/// The second finding, found with the files written here: an edge on a
/// wide cone that passes close to its apex, given by the writer as a
/// B-spline of few long spans (the exact parabola of a cone of 81.5°
/// half-angle cut 0.015 from its apex, one rational quadratic span), is
/// not proven on the cone by `geom::certify` within its 4096 pieces at
/// any target up to 64 times the cap, while the same curve in spans of a
/// unit of its parameter is. Exact, it is accepted on 1025 places within
/// the tolerance (counted as unproven); lifted 1e-5 off the plane, it was
/// refused "could not be proven within the cap", without saying what
/// would read. The refusal now says how far the places lie and that a
/// document tolerance of at least that accepts it, and it does.
#[test]
fn an_edge_the_prover_gives_out_on_is_refused_saying_what_reads() {
    let (h, c) = (0.3, 1.9);
    let frame = frames(1.0)[1].1;
    let exact = cut_cone(Cut::Parabola, 2.0, h, c, frame, 1.0, Form::Spline(1, 0.0));
    let read_exact = read(&exact, 1.0).unwrap();
    let (v, a) = measures(&read_exact.body);
    let (ev, ea) = closed_form(Cut::Parabola, 2.0, h, c);
    assert!(
        relative(v, ev) < 1e-9 && relative(a, ea) < 1e-9,
        "{v} / {ev}, {a} / {ea}"
    );
    assert!(read_exact.report.unproven_edges <= 1);
    // The same curve as the reader makes a parabola (spans of at most a
    // unit of its parameter): proven.
    let pieces = cut_cone(Cut::Parabola, 2.0, h, c, frame, 1.0, Form::Plain);
    assert_eq!(read(&pieces, 1.0).unwrap().report.unproven_edges, 0);
    let lifted = cut_cone(Cut::Parabola, 2.0, h, c, frame, 1.0, Form::Spline(1, 1e-5));
    let e = read(&lifted, 1.0).unwrap_err();
    assert!(
        e.contains("could not be proven within the cap") && e.contains("document.setTolerance"),
        "{e}"
    );
    let gap: f64 = e
        .split("lies at most ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|x| x.parse().ok())
        .unwrap_or_else(|| panic!("no gap in {e}"));
    assert!(gap > 1e-6 && gap < 1e-5, "{gap}");
    // With the tolerance the message names, it reads.
    let looser = Options {
        tolerance: Tolerance::new(gap * 1.01, 1e-9).unwrap(),
        loosest: 1e-3,
        ..options(1.0)
    };
    let parsed = parse(lifted.as_bytes()).unwrap();
    let done = parsed.build(0, &looser).unwrap();
    let (v, _) = measures(&done.body);
    assert!(relative(v, ev) < 1e-4, "{v} / {ev}");
    assert!(done.report.unproven_edges <= 1);
}
