//! STEP import of assemblies and colours (docs/design/step-import.md,
//! slice 2): a part the export writes, placed by hand-made assembly
//! structure the way other writers place parts (`NEXT_ASSEMBLY_USAGE_
//! OCCURRENCE` with `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` and an
//! `ITEM_DEFINED_TRANSFORMATION`, either order of its frames; nested
//! assemblies; `MAPPED_ITEM`), read back as one body per placement where
//! the transformation puts it; the colours the export writes read back;
//! hostile assemblies refused, bounded, never a panic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_io::{StepBody, StepOptions, step_model};
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_step::{Entity, Instance, Param, Record, StepFile};
use craft3d_stepread::{Imported, Options, parse};
use craft3d_topo::Body;

fn tolerance() -> Tolerance {
    Tolerance::new(1e-6, 1e-9).unwrap()
}

fn options() -> Options {
    Options {
        metres_per_unit: 1e-3,
        tolerance: tolerance(),
        loosest: 1e-3,
    }
}

fn part() -> Body {
    let frame = Frame::new(Vec3::new(1.0, 2.0, 3.0), Vec3::Z, Vec3::X).unwrap();
    craft3d_kernel::primitives::block(&frame, 4.0, 2.0, 1.0).unwrap()
}

fn exported(body: &Body, color: Option<[u8; 3]>, faces: &[Option<[u8; 3]>]) -> StepFile {
    let options = StepOptions {
        tolerance: tolerance(),
        ..StepOptions::default()
    };
    let mut item = StepBody::new("part", body);
    item.color = color;
    item.face_colors = faces;
    step_model(&[item], &options).unwrap().0
}

fn read(file: &StepFile) -> Result<Vec<Imported>, String> {
    let text = craft3d_step::write(file).unwrap();
    let parsed = parse(text.as_bytes()).map_err(|e| e.to_string())?;
    (0..parsed.placed().len())
        .map(|i| parsed.build(i, &options()).map_err(|e| e.to_string()))
        .collect()
}

/// Adds instances to a file.
struct Adder {
    file: StepFile,
    next: u64,
}

impl Adder {
    fn new(file: StepFile) -> Self {
        let next = file.data.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        Self { file, next }
    }

    fn add(&mut self, keyword: &str, params: Vec<Param>) -> u64 {
        self.complex(vec![Record::new(keyword, params)])
    }

    fn complex(&mut self, records: Vec<Record>) -> u64 {
        let id = self.next;
        self.next += 1;
        let entity = if records.len() == 1 {
            Entity::Simple(records.into_iter().next().unwrap())
        } else {
            Entity::Complex(records)
        };
        self.file.data.push(Instance { id, entity });
        id
    }

    fn find(&self, keyword: &str) -> u64 {
        self.file
            .data
            .iter()
            .find(|i| i.entity.is(keyword))
            .unwrap_or_else(|| panic!("no {keyword}"))
            .id
    }

    fn placement(&mut self, f: &Frame) -> u64 {
        let p = f.origin();
        let origin = self.add(
            "CARTESIAN_POINT",
            vec![Param::string(""), Param::reals([p.x, p.y, p.z])],
        );
        let z = f.z();
        let z = self.add(
            "DIRECTION",
            vec![Param::string(""), Param::reals([z.x, z.y, z.z])],
        );
        let x = f.x();
        let x = self.add(
            "DIRECTION",
            vec![Param::string(""), Param::reals([x.x, x.y, x.z])],
        );
        self.add(
            "AXIS2_PLACEMENT_3D",
            vec![
                Param::string(""),
                Param::Ref(origin),
                Param::Ref(z),
                Param::Ref(x),
            ],
        )
    }

    /// A product with a shape representation listing `items`, in the
    /// part's context: (product definition, shape representation).
    fn product(&mut self, name: &str, items: Vec<u64>) -> (u64, u64) {
        let context = self.find("GLOBAL_UNIT_ASSIGNED_CONTEXT");
        let pc = self.find("PRODUCT_CONTEXT");
        let dc = self.find("PRODUCT_DEFINITION_CONTEXT");
        let product = self.add(
            "PRODUCT",
            vec![
                Param::string(name),
                Param::string(name),
                Param::string(""),
                Param::refs([pc]),
            ],
        );
        let formation = self.add(
            "PRODUCT_DEFINITION_FORMATION",
            vec![Param::string(""), Param::string(""), Param::Ref(product)],
        );
        let definition = self.add(
            "PRODUCT_DEFINITION",
            vec![
                Param::string("design"),
                Param::string(""),
                Param::Ref(formation),
                Param::Ref(dc),
            ],
        );
        let shape = self.add(
            "PRODUCT_DEFINITION_SHAPE",
            vec![Param::string(""), Param::string(""), Param::Ref(definition)],
        );
        let rep = self.add(
            "SHAPE_REPRESENTATION",
            vec![Param::string(name), Param::refs(items), Param::Ref(context)],
        );
        self.add(
            "SHAPE_DEFINITION_REPRESENTATION",
            vec![Param::Ref(shape), Param::Ref(rep)],
        );
        (definition, rep)
    }

    /// Adds `item` to the items of representation `rep`.
    fn list(&mut self, rep: u64, item: u64) {
        let instance = self.file.data.iter_mut().find(|i| i.id == rep).unwrap();
        let Entity::Simple(record) = &mut instance.entity else {
            panic!("a complex representation")
        };
        let Param::List(items) = &mut record.params[1] else {
            panic!("no items")
        };
        items.push(Param::Ref(item));
    }

    /// Places `child` (definition, representation, its origin frame) in
    /// `parent` at `at`, as an occurrence named `tag`; `parent_first`
    /// writes the parent's frame as `transform_item_1`.
    #[allow(clippy::too_many_arguments)]
    fn occurrence(
        &mut self,
        parent: (u64, u64),
        child: (u64, u64, u64),
        at: &Frame,
        tag: &str,
        parent_first: bool,
    ) -> u64 {
        let frame = self.placement(at);
        self.list(parent.1, frame);
        let (a, b) = if parent_first {
            (frame, child.2)
        } else {
            (child.2, frame)
        };
        let idt = self.add(
            "ITEM_DEFINED_TRANSFORMATION",
            vec![
                Param::string(""),
                Param::string(""),
                Param::Ref(a),
                Param::Ref(b),
            ],
        );
        let rrwt = self.complex(vec![
            Record::new(
                "REPRESENTATION_RELATIONSHIP",
                vec![
                    Param::string(""),
                    Param::string(""),
                    Param::Ref(child.1),
                    Param::Ref(parent.1),
                ],
            ),
            Record::new(
                "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION",
                vec![Param::Ref(idt)],
            ),
            Record::new("SHAPE_REPRESENTATION_RELATIONSHIP", vec![]),
        ]);
        let nauo = self.add(
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE",
            vec![
                Param::string(tag),
                Param::string(tag),
                Param::string(""),
                Param::Ref(parent.0),
                Param::Ref(child.0),
                Param::Omitted,
            ],
        );
        let pds = self.add(
            "PRODUCT_DEFINITION_SHAPE",
            vec![Param::string(""), Param::string(""), Param::Ref(nauo)],
        );
        self.add(
            "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION",
            vec![Param::Ref(rrwt), Param::Ref(pds)],
        );
        rrwt
    }

    /// The part of the exported file as a child: its definition, a shape
    /// representation with an origin frame, related to its B-rep's
    /// representation without a transformation.
    fn part_as_child(&mut self) -> (u64, u64, u64) {
        let definition = self.find("PRODUCT_DEFINITION");
        let brep = self.find("ADVANCED_BREP_SHAPE_REPRESENTATION");
        let context = self.find("GLOBAL_UNIT_ASSIGNED_CONTEXT");
        let origin = self.placement(&Frame::WORLD);
        let rep = self.add(
            "SHAPE_REPRESENTATION",
            vec![
                Param::string("part"),
                Param::refs([origin]),
                Param::Ref(context),
            ],
        );
        self.add(
            "SHAPE_REPRESENTATION_RELATIONSHIP",
            vec![
                Param::string(""),
                Param::string(""),
                Param::Ref(rep),
                Param::Ref(brep),
            ],
        );
        (definition, rep, origin)
    }
}

fn frames() -> Vec<Frame> {
    vec![
        Frame::new(Vec3::new(10.0, 0.0, 0.0), Vec3::Z, Vec3::Y).unwrap(),
        Frame::new(Vec3::new(-5.0, 20.0, 7.0), Vec3::X, Vec3::Y).unwrap(),
        Frame::new(
            Vec3::new(0.5, -3.0, 40.0),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

fn centroid(body: &Body) -> Vec3 {
    craft3d_topo::mass_properties(body)
        .unwrap()
        .centroid
        .unwrap()
}

fn volume(body: &Body) -> f64 {
    craft3d_topo::mass_properties(body).unwrap().volume
}

/// The part placed three times in an assembly: three bodies, each where
/// its frame puts the part (the part's own centroid carried by the
/// frame), the same volume, named by the assembly's product and the
/// occurrence, whichever order the transformation lists its frames in.
#[test]
fn a_part_placed_three_times_reads_as_three_bodies_where_they_are_placed() {
    let body = part();
    let own = centroid(&body);
    for parent_first in [false, true] {
        let mut adder = Adder::new(exported(&body, None, &[]));
        let child = adder.part_as_child();
        let origin = adder.placement(&Frame::WORLD);
        let root = adder.product("bracket", vec![origin]);
        for (k, f) in frames().iter().enumerate() {
            adder.occurrence(root, child, f, &format!("{}", k + 1), parent_first);
        }
        let text = craft3d_step::write(&adder.file).unwrap();
        let parsed = parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.items().len(), 1);
        let paths: Vec<&str> = parsed.placed().iter().map(|p| p.path.as_str()).collect();
        assert_eq!(
            paths,
            ["bracket/part#1", "bracket/part#2", "bracket/part#3"]
        );
        let bodies = read(&adder.file).unwrap();
        assert_eq!(bodies.len(), 3);
        for (imported, f) in bodies.iter().zip(frames()) {
            let expected = Transform::from_frame(&f).point(own);
            let got = centroid(&imported.body);
            assert!(
                got.distance(expected) < 1e-9,
                "parent first {parent_first}: {got:?} != {expected:?}"
            );
            assert!((volume(&imported.body) - volume(&body)).abs() < 1e-9);
            assert!(imported.path.starts_with("bracket/part#"));
        }
        // The same bytes again: the same placements and bodies, bit for bit.
        let again = read(&adder.file).unwrap();
        for (a, b) in bodies.iter().zip(&again) {
            assert_eq!(a.path, b.path);
            assert_eq!(
                format!("{:?}", a.body.vertices),
                format!("{:?}", b.body.vertices)
            );
        }
    }
}

/// A part placed where it is (an identity placement, which slice 1
/// accepted) reads bit for bit as the part alone: documents saved before
/// regenerate the same bodies.
#[test]
fn an_identity_placement_leaves_the_body_as_it_was() {
    let body = part();
    let alone = read(&exported(&body, None, &[])).unwrap();
    let mut adder = Adder::new(exported(&body, None, &[]));
    let child = adder.part_as_child();
    let origin = adder.placement(&Frame::WORLD);
    let root = adder.product("same", vec![origin]);
    adder.occurrence(root, child, &Frame::WORLD, "1", false);
    let placed = read(&adder.file).unwrap();
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].path, "same/part#1");
    assert_eq!(
        format!("{:?}", placed[0].body),
        format!("{:?}", alone[0].body)
    );
}

/// A sub-assembly holding the part twice, placed twice in the root: four
/// bodies, the transformations composed from the part up.
#[test]
fn nested_assemblies_compose_their_placements() {
    let body = part();
    let own = centroid(&body);
    let mut adder = Adder::new(exported(&body, None, &[]));
    let child = adder.part_as_child();
    let sub_origin = adder.placement(&Frame::WORLD);
    let sub = adder.product("arm", vec![sub_origin]);
    let f = frames();
    adder.occurrence(sub, child, &f[0], "a", false);
    adder.occurrence(sub, child, &f[1], "b", false);
    let root_origin = adder.placement(&Frame::WORLD);
    let root = adder.product("robot", vec![root_origin]);
    let sub_child = (sub.0, sub.1, sub_origin);
    adder.occurrence(root, sub_child, &f[2], "left", false);
    adder.occurrence(root, sub_child, &f[1], "right", true);
    let bodies = read(&adder.file).unwrap();
    assert_eq!(bodies.len(), 4);
    let mut expected = Vec::new();
    for (outer, outer_tag) in [(&f[2], "left"), (&f[1], "right")] {
        for (inner, inner_tag) in [(&f[0], "a"), (&f[1], "b")] {
            let t = Transform::from_frame(inner).then(&Transform::from_frame(outer));
            expected.push((
                format!("robot/arm#{outer_tag}/part#{inner_tag}"),
                t.point(own),
            ));
        }
    }
    let mut got: Vec<(String, Vec3)> = bodies
        .iter()
        .map(|b| (b.path.clone(), centroid(&b.body)))
        .collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    for ((path, at), (want_path, want)) in got.iter().zip(&expected) {
        assert_eq!(path, want_path);
        assert!(at.distance(*want) < 1e-9, "{path}: {at:?} != {want:?}");
    }
}

/// A part placed by `MAPPED_ITEM`s in the root's representation.
#[test]
fn mapped_items_place_a_part() {
    let body = part();
    let own = centroid(&body);
    let mut adder = Adder::new(exported(&body, None, &[]));
    let (_, part_rep, part_origin) = adder.part_as_child();
    let mut mapped = Vec::new();
    let map = adder.add(
        "REPRESENTATION_MAP",
        vec![Param::Ref(part_origin), Param::Ref(part_rep)],
    );
    for (k, f) in frames().iter().enumerate() {
        let target = adder.placement(f);
        mapped.push(adder.add(
            "MAPPED_ITEM",
            vec![
                Param::string(&format!("copy{k}")),
                Param::Ref(map),
                Param::Ref(target),
            ],
        ));
    }
    let root = adder.product("plate", mapped);
    let _ = root;
    let bodies = read(&adder.file).unwrap();
    assert_eq!(bodies.len(), 3);
    for (k, (imported, f)) in bodies.iter().zip(frames()).enumerate() {
        let expected = Transform::from_frame(&f).point(own);
        assert!(centroid(&imported.body).distance(expected) < 1e-9);
        assert_eq!(imported.path, format!("plate/copy{k}"));
    }
}

/// An assembly in metres placing a part in millimetres: each frame is
/// read in its own representation's unit.
#[test]
fn a_part_in_another_unit_than_its_assembly_lands_where_it_should() {
    let body = part();
    let own = centroid(&body);
    let mut adder = Adder::new(exported(&body, None, &[]));
    let child = adder.part_as_child();
    // A context in metres for the assembly.
    let metre = adder.complex(vec![
        Record::new("LENGTH_UNIT", vec![]),
        Record::new("NAMED_UNIT", vec![Param::Derived]),
        Record::new("SI_UNIT", vec![Param::Omitted, Param::enumeration("METRE")]),
    ]);
    let context = adder.complex(vec![
        Record::new("GEOMETRIC_REPRESENTATION_CONTEXT", vec![Param::Integer(3)]),
        Record::new("GLOBAL_UNIT_ASSIGNED_CONTEXT", vec![Param::refs([metre])]),
        Record::new(
            "REPRESENTATION_CONTEXT",
            vec![Param::string(""), Param::string("")],
        ),
    ]);
    let origin = adder.placement(&Frame::WORLD);
    let root = adder.product("frame", vec![origin]);
    // Point the root's representation at the context in metres.
    for instance in &mut adder.file.data {
        if instance.id == root.1
            && let Entity::Simple(record) = &mut instance.entity
        {
            record.params[2] = Param::Ref(context);
        }
    }
    // 0.25 m along x.
    let at = Frame::new(Vec3::new(0.25, 0.0, 0.0), Vec3::Z, Vec3::X).unwrap();
    adder.occurrence(root, child, &at, "1", false);
    let bodies = read(&adder.file).unwrap();
    assert_eq!(bodies.len(), 1);
    let expected = own + Vec3::new(250.0, 0.0, 0.0);
    let got = centroid(&bodies[0].body);
    assert!(got.distance(expected) < 1e-9, "{got:?} != {expected:?}");
}

/// Assemblies a hostile file may hold: refused naming why, bounded,
/// never a panic.
#[test]
fn hostile_assemblies_are_refused_and_bounded() {
    let body = part();
    let base = exported(&body, None, &[]);
    let refused = |file: &StepFile| -> String {
        let text = craft3d_step::write(file).unwrap();
        match parse(text.as_bytes()) {
            Err(e) => e.to_string(),
            Ok(parsed) => {
                for i in 0..parsed.placed().len() {
                    if let Err(e) = parsed.build(i, &options()) {
                        return e.to_string();
                    }
                }
                panic!("read: {} placements", parsed.placed().len())
            }
        }
    };

    // An assembly that contains itself, through another.
    let mut adder = Adder::new(base.clone());
    let child = adder.part_as_child();
    let a_origin = adder.placement(&Frame::WORLD);
    let a = adder.product("a", vec![a_origin]);
    let b_origin = adder.placement(&Frame::WORLD);
    let b = adder.product("b", vec![b_origin]);
    adder.occurrence(a, child, &frames()[0], "1", false);
    adder.occurrence(b, (a.0, a.1, a_origin), &frames()[1], "2", false);
    adder.occurrence(a, (b.0, b.1, b_origin), &frames()[2], "3", false);
    let e = refused(&adder.file);
    assert!(e.contains("contains itself"), "{e}");

    // A chain of assemblies deeper than the limit.
    let mut adder = Adder::new(base.clone());
    let mut below = adder.part_as_child();
    for level in 0..craft3d_stepread::MAX_DEPTH + 2 {
        let origin = adder.placement(&Frame::WORLD);
        let up = adder.product(&format!("level{level}"), vec![origin]);
        adder.occurrence(up, below, &frames()[0], "1", false);
        below = (up.0, up.1, origin);
    }
    let e = refused(&adder.file);
    assert!(e.contains("nested more than"), "{e}");

    // A bomb: every level places the one below twice, 2^16 placements.
    let mut adder = Adder::new(base.clone());
    let mut below = adder.part_as_child();
    for level in 0..16 {
        let origin = adder.placement(&Frame::WORLD);
        let up = adder.product(&format!("level{level}"), vec![origin]);
        adder.occurrence(up, below, &frames()[0], "1", false);
        adder.occurrence(up, below, &frames()[1], "2", false);
        below = (up.0, up.1, origin);
    }
    let started = std::time::Instant::now();
    let e = refused(&adder.file);
    assert!(e.contains("more than"), "{e}");
    assert!(started.elapsed().as_secs() < 20);

    // A placement beyond what a double holds once converted.
    let mut adder = Adder::new(base.clone());
    let child = adder.part_as_child();
    let origin = adder.placement(&Frame::WORLD);
    let root = adder.product("far", vec![origin]);
    let far = Frame::new(Vec3::new(1e308, 1e308, 0.0), Vec3::Z, Vec3::X).unwrap();
    adder.occurrence(root, child, &far, "1", false);
    let e = refused(&adder.file);
    assert!(e.contains("rounding is more than the tolerance"), "{e}");

    // A placement that scales: a transformation operator with scale 2.
    let mut adder = Adder::new(base.clone());
    let (_, part_rep, _) = adder.part_as_child();
    let origin = adder.placement(&Frame::WORLD);
    let root = adder.product("scaled", vec![origin]);
    let point = adder.add(
        "CARTESIAN_POINT",
        vec![Param::string(""), Param::reals([0.0, 0.0, 0.0])],
    );
    let operator = adder.add(
        "CARTESIAN_TRANSFORMATION_OPERATOR_3D",
        vec![
            Param::string(""),
            Param::string(""),
            Param::string(""),
            Param::Omitted,
            Param::Omitted,
            Param::Ref(point),
            Param::Real(2.0),
            Param::Omitted,
        ],
    );
    adder.complex(vec![
        Record::new(
            "REPRESENTATION_RELATIONSHIP",
            vec![
                Param::string(""),
                Param::string(""),
                Param::Ref(part_rep),
                Param::Ref(root.1),
            ],
        ),
        Record::new(
            "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION",
            vec![Param::Ref(operator)],
        ),
        Record::new("SHAPE_REPRESENTATION_RELATIONSHIP", vec![]),
    ]);
    let e = refused(&adder.file);
    assert!(e.contains("scales"), "{e}");

    // References of the wrong kinds in the assembly structure.
    let mut adder = Adder::new(base);
    let child = adder.part_as_child();
    let origin = adder.placement(&Frame::WORLD);
    let root = adder.product("bad", vec![origin]);
    let rrwt = adder.occurrence(root, child, &frames()[0], "1", false);
    for instance in &mut adder.file.data {
        if instance.id == rrwt
            && let Entity::Complex(records) = &mut instance.entity
        {
            records[1].params[0] = Param::Ref(origin);
        }
    }
    let e = refused(&adder.file);
    assert!(e.contains("not read") || e.contains("is a"), "{e}");
}

/// The colours the export writes read back: a body's colour; a body
/// whose faces alone are coloured takes the colour most of them have.
#[test]
fn colours_read_back_per_solid() {
    let body = part();
    let bodies = read(&exported(&body, Some([200, 100, 50]), &[])).unwrap();
    assert_eq!(bodies[0].color, Some([200, 100, 50]));
    let mut faces = vec![Some([10, 20, 30]); body.faces.len()];
    faces[0] = Some([1, 2, 3]);
    let bodies = read(&exported(&body, None, &faces)).unwrap();
    assert_eq!(bodies[0].color, Some([10, 20, 30]));
    let bodies = read(&exported(&body, None, &[])).unwrap();
    assert_eq!(bodies[0].color, None);
    // Some faces styled, others not: the body has no colour (the others
    // have the solid's, which the file does not give); the styled faces
    // keep their own (2026-10-22, face colours).
    let mut some = vec![None; body.faces.len()];
    some[1] = Some([10, 20, 30]);
    some[2] = Some([10, 20, 30]);
    let file = exported(&body, None, &some);
    let bodies = read(&file).unwrap();
    assert_eq!(bodies[0].color, None);
    let text = craft3d_step::write(&file).unwrap();
    let parsed = parse(text.as_bytes()).unwrap();
    let built = parsed.build(0, &options()).unwrap();
    assert_eq!(parsed.face_colors(&built.face_ids), some);
    // A colour that is no colour (outside 0 to 1) is left out, the body
    // read.
    let mut file = exported(&body, Some([200, 100, 50]), &[]);
    for instance in &mut file.data {
        if let Entity::Simple(record) = &mut instance.entity
            && record.keyword == "COLOUR_RGB"
        {
            record.params[1] = Param::Real(7.0);
        }
    }
    let bodies = read(&file).unwrap();
    assert_eq!(bodies[0].color, None);
}
