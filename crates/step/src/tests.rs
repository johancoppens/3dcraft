use super::*;

fn header() -> Header {
    Header {
        description: vec!["a test".to_string()],
        implementation_level: "2;1".to_string(),
        name: "test.step".to_string(),
        time_stamp: "2026-10-13T12:00:00".to_string(),
        author: vec!["someone".to_string(), "someone else".to_string()],
        organization: Vec::new(),
        preprocessor_version: "craft3d-step".to_string(),
        originating_system: "3DCraft".to_string(),
        authorization: String::new(),
        schemas: vec!["AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }".to_string()],
    }
}

/// A file with one of everything the model can hold.
fn sample() -> StepFile {
    let mut file = StepFile::new(&header());
    let point = file.add(
        "CARTESIAN_POINT",
        vec![Param::string("origin"), Param::reals([0.0, -1.5, 2.5e-9])],
    );
    let direction = file.add(
        "DIRECTION",
        vec![Param::string(""), Param::reals([0.0, 0.0, 1.0])],
    );
    let placement = file.add(
        "AXIS2_PLACEMENT_3D",
        vec![
            Param::string("it's"),
            Param::Ref(point),
            Param::Ref(direction),
            Param::Omitted,
        ],
    );
    let unit = file.add_complex(vec![
        Record::new("LENGTH_UNIT", vec![]),
        Record::new("NAMED_UNIT", vec![Param::Derived]),
        Record::new(
            "SI_UNIT",
            vec![Param::enumeration("MILLI"), Param::enumeration("METRE")],
        ),
    ]);
    file.add(
        "UNCERTAINTY_MEASURE_WITH_UNIT",
        vec![
            Param::typed("LENGTH_MEASURE", Param::Real(1e-7)),
            Param::Ref(unit),
            Param::string("distance_accuracy_value"),
            Param::string("back\\slash, caf\u{e9}, \u{4e16}\u{754c}, \u{1f600}, line\nbreak"),
        ],
    );
    file.add(
        "!USER_DEFINED",
        vec![
            Param::Integer(-42),
            Param::Integer(i64::MAX),
            Param::boolean(true),
            Param::Binary("1F0A".to_string()),
            Param::List(vec![
                Param::List(vec![Param::Ref(placement), Param::List(vec![])]),
                Param::refs([point, direction]),
            ]),
            Param::List(vec![]),
        ],
    );
    file
}

#[test]
fn reals_are_written_the_part_21_way() {
    for (value, text) in [
        (0.0, "0."),
        (-0.0, "-0."),
        (1.0, "1."),
        (-25.4, "-25.4"),
        (0.25, "0.25"),
        (1e-7, "1.E-7"),
        (1.5e-7, "1.5E-7"),
        (1e21, "1.E21"),
        (123456.0, "123456."),
        (0.1 + 0.2, "0.30000000000000004"),
        (f64::MAX, "1.7976931348623157E308"),
        (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
        (5e-324, "5.E-324"),
    ] {
        assert_eq!(format_real(value).as_deref(), Some(text));
    }
    assert_eq!(format_real(f64::NAN), None);
    assert_eq!(format_real(f64::INFINITY), None);
    assert_eq!(format_real(f64::NEG_INFINITY), None);
}

/// A small deterministic generator (xorshift64*).
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[test]
fn every_real_reads_back_bit_for_bit() {
    let mut random = Random(0x3DC2_AF70_0000_0001);
    let mut file = StepFile::new(&header());
    let mut values = vec![0.0, -0.0, 1.0, f64::MAX, f64::MIN_POSITIVE, 5e-324, -1e-300];
    while values.len() < 20_000 {
        // Any bit pattern that is a finite number: all exponents, all
        // mantissas, subnormals included.
        let x = f64::from_bits(random.next());
        if x.is_finite() {
            values.push(x);
        }
    }
    for chunk in values.chunks(3) {
        file.add("P", vec![Param::reals(chunk.iter().copied())]);
    }
    let text = write(&file).unwrap();
    assert!(text.is_ascii());
    let back = read(text.as_bytes()).unwrap();
    let read_values: Vec<u64> = back
        .data
        .iter()
        .flat_map(|i| match &i.entity {
            Entity::Simple(r) => r.params[0].as_list().unwrap().to_vec(),
            Entity::Complex(_) => panic!(),
        })
        .map(|p| p.as_real().unwrap().to_bits())
        .collect();
    let written: Vec<u64> = values.iter().map(|x| x.to_bits()).collect();
    assert_eq!(read_values, written);
}

#[test]
fn a_file_reads_back_as_the_same_model() {
    let file = sample();
    let text = write(&file).unwrap();
    assert!(text.is_ascii(), "{text}");
    let back = read(text.as_bytes()).unwrap();
    assert_eq!(back, file);
    assert_eq!(back.header_values().unwrap(), {
        let mut h = header();
        // An empty list of strings is written as one empty string.
        h.organization = vec![String::new()];
        h
    });
    // And writing what was read gives the same text.
    assert_eq!(write(&back).unwrap(), text);
    assert_eq!(back.unresolved_reference(), None);
    assert_eq!(back.reference_cycle(), None);
    assert_eq!(back.count("SI_UNIT"), 1);
    assert_eq!(back.counts()["LENGTH_UNIT"], 1);
}

#[test]
fn the_text_is_what_the_standard_describes() {
    let text = write(&sample()).unwrap();
    let expected = "\
ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('a test'),'2;1');
FILE_NAME('test.step','2026-10-13T12:00:00',('someone','someone else'),(''),'craft3d-step',
  '3DCraft','');
FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));
ENDSEC;
DATA;
#1=CARTESIAN_POINT('origin',(0.,-1.5,2.5E-9));
#2=DIRECTION('',(0.,0.,1.));
#3=AXIS2_PLACEMENT_3D('it''s',#1,#2,$);
#4=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));
#5=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-7),#4,'distance_accuracy_value',
  'back\\\\slash, caf\\X2\\00E9\\X0\\, \\X2\\4E16754C\\X0\\, \\X4\\0001F600\\X0\\, line\\X2\\000A\\X0\\break');
#6=!USER_DEFINED(-42,9223372036854775807,.T.,\"1F0A\",((#3,()),(#1,#2)),());
ENDSEC;
END-ISO-10303-21;
";
    assert_eq!(text, expected);
}

#[test]
fn strings_decode_every_directive() {
    let read_string = |body: &str| -> Result<String, ReadError> {
        let text =
            format!("ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('{body}');ENDSEC;END-ISO-10303-21;");
        let file = read(text.as_bytes())?;
        Ok(file.data[0].entity.records()[0].params[0]
            .as_str()
            .unwrap()
            .to_string())
    };
    assert_eq!(read_string("plain").unwrap(), "plain");
    assert_eq!(read_string("it''s").unwrap(), "it's");
    assert_eq!(read_string("a\\\\b").unwrap(), "a\\b");
    assert_eq!(read_string("caf\\X\\E9").unwrap(), "caf\u{e9}");
    assert_eq!(read_string("caf\\S\\i").unwrap(), "caf\u{e9}");
    assert_eq!(read_string("\\PA\\caf\\S\\i").unwrap(), "caf\u{e9}");
    // Another ISO 8859 part: not decoded, and it says so.
    assert_eq!(read_string("\\PE\\\\S\\i").unwrap(), "\u{fffd}");
    assert_eq!(
        read_string("\\X2\\4E16754C\\X0\\!").unwrap(),
        "\u{4e16}\u{754c}!"
    );
    assert_eq!(read_string("\\X4\\0001F600\\X0\\").unwrap(), "\u{1f600}");
    // A surrogate pair in \X2\ is one character.
    assert_eq!(read_string("\\X2\\D83DDE00\\X0\\").unwrap(), "\u{1f600}");
    // Raw UTF-8 (edition 3) and raw ISO 8859-1 (older writers).
    assert_eq!(read_string("caf\u{e9}").unwrap(), "caf\u{e9}");
    let latin1 = b"ISO-10303-21;HEADER;ENDSEC;DATA;#1=A('caf\xE9');ENDSEC;END-ISO-10303-21;";
    let file = read(latin1).unwrap();
    assert_eq!(
        file.data[0].entity.records()[0].params[0].as_str(),
        Some("caf\u{e9}")
    );

    for bad in [
        "\\",
        "\\Q",
        "\\X\\",
        "\\X\\G1",
        "\\X2\\00E",
        "\\X2\\00E9",
        "\\X2\\00E9\\X1\\",
        "\\X2\\D83D\\X0\\",
        "\\X2\\DE00\\X0\\",
        "\\X4\\00110000\\X0\\",
        "\\X4\\0000D800\\X0\\",
        "\\S\\",
        "tab\there",
    ] {
        let error = read_string(bad).unwrap_err();
        assert!(
            matches!(error.kind, ReadErrorKind::BadString { .. }),
            "{bad}: {error}"
        );
    }
}

#[test]
fn comments_and_white_space_are_dropped() {
    let text = "ISO-10303-21 ;\r\n/* a comment */HEADER;\n/**/ENDSEC;DATA;\n\
                #10 = /* inside */ A ( 1 , /* (#99) */ 2.5 , ( ) ) ;\n\
                #20=(B()\n C(#10));\nENDSEC;\nEND-ISO-10303-21;\n\x1A";
    let file = read(text.as_bytes()).unwrap();
    assert_eq!(file.data.len(), 2);
    assert_eq!(
        file.data[0],
        Instance {
            id: 10,
            entity: Entity::Simple(Record::new(
                "A",
                vec![Param::Integer(1), Param::Real(2.5), Param::List(vec![])]
            )),
        }
    );
    assert!(file.data[1].entity.is("B") && file.data[1].entity.is("C"));
    assert!(!file.data[1].entity.is("A"));
    // Numbers as other writers spell them.
    let text =
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(+1,-0,1.,+1.5E+2,1.e-3,-.5);ENDSEC;END-ISO-10303-21;";
    let error = read(text.as_bytes()).unwrap_err();
    // `-.5` has no digit before the point: not a Part 21 real.
    assert!(
        matches!(error.kind, ReadErrorKind::BadNumber { .. }),
        "{error}"
    );
    let text = text.replace(",-.5", "");
    let file = read(text.as_bytes()).unwrap();
    assert_eq!(
        file.data[0].entity.records()[0].params,
        vec![
            Param::Integer(1),
            Param::Integer(0),
            Param::Real(1.0),
            Param::Real(150.0),
            Param::Real(0.001)
        ]
    );
}

fn wrap(data: &str) -> String {
    format!("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n{data}\nENDSEC;\nEND-ISO-10303-21;\n")
}

#[test]
fn hostile_input_is_an_error_with_its_position() {
    use ReadErrorKind as K;
    type Case = (String, fn(&K) -> bool, usize);
    let cases: Vec<Case> = vec![
        (String::new(), |k| matches!(k, K::UnexpectedEnd { .. }), 1),
        ("STEP;".into(), |k| matches!(k, K::Unexpected { .. }), 1),
        (
            "ISO-10303-21;".into(),
            |k| matches!(k, K::UnexpectedEnd { .. }),
            1,
        ),
        (
            "ISO-10303-21;HEADER;A(;".into(),
            |k| matches!(k, K::Unexpected { .. }),
            1,
        ),
        (
            wrap("#1=A(#2);"),
            |k| matches!(k, K::UnresolvedReference { from: 1, to: 2 }),
            5,
        ),
        (
            wrap("#1=A();\n#1=B();"),
            |k| matches!(k, K::DuplicateInstance { id: 1 }),
            6,
        ),
        (wrap("#0=A();"), |k| matches!(k, K::BadInstanceName), 5),
        (wrap("#=A();"), |k| matches!(k, K::BadInstanceName), 5),
        (
            wrap("#99999999999999999999999=A();"),
            |k| matches!(k, K::BadInstanceName),
            5,
        ),
        (
            wrap("#1=A(99999999999999999999);"),
            |k| matches!(k, K::BadNumber { .. }),
            5,
        ),
        (
            wrap("#1=A(1.E999);"),
            |k| matches!(k, K::BadNumber { .. }),
            5,
        ),
        (wrap("#1=A(1.E);"), |k| matches!(k, K::BadNumber { .. }), 5),
        (wrap("#1=A(-);"), |k| matches!(k, K::BadNumber { .. }), 5),
        (
            wrap("#1=A('open);"),
            |k| matches!(k, K::BadString { .. }),
            5,
        ),
        (
            wrap("#1=A(\"0G\");"),
            |k| matches!(k, K::BadString { .. }),
            5,
        ),
        (wrap("#1=A(\"\");"), |k| matches!(k, K::BadString { .. }), 5),
        (wrap("#1=A(.T);"), |k| matches!(k, K::BadEnum), 5),
        (wrap("#1=A(.t.);"), |k| matches!(k, K::BadEnum), 5),
        (wrap("#1=A(..);"), |k| matches!(k, K::BadEnum), 5),
        (wrap("#1=a();"), |k| matches!(k, K::BadKeyword), 5),
        (wrap("#1=A-B();"), |k| matches!(k, K::BadKeyword), 5),
        (wrap("#1=A(b(1));"), |k| matches!(k, K::BadKeyword), 5),
        (
            wrap("#1=A(B(1,2));"),
            |k| matches!(k, K::Unexpected { .. }),
            5,
        ),
        (wrap("#1=A(B());"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("#1=A(1 2);"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("#1=A(1,);"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("#1=A(,1);"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("#1=A(1;"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("#1=A(1)"), |k| matches!(k, K::Unexpected { .. }), 6),
        (wrap("#1=();"), |k| matches!(k, K::Unexpected { .. }), 5),
        (
            wrap("#1=(A() 5);"),
            |k| matches!(k, K::Unexpected { .. }),
            5,
        ),
        (wrap("#1 A();"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("A();"), |k| matches!(k, K::Unexpected { .. }), 5),
        (wrap("#1=A(@);"), |k| matches!(k, K::BadCharacter), 5),
        (wrap("#1=A(\x00);"), |k| matches!(k, K::BadCharacter), 5),
        (wrap("#1=A(); /* open"), |k| matches!(k, K::OpenComment), 5),
        (
            wrap("#1=&SCOPE #2=A(); ENDSCOPE B();"),
            |k| matches!(k, K::Unsupported { .. }),
            5,
        ),
        (
            "ISO-10303-21;HEADER;ENDSEC;DATA;ENDSEC;DATA;ENDSEC;END-ISO-10303-21;".into(),
            |k| matches!(k, K::Unsupported { .. }),
            1,
        ),
        (
            "ISO-10303-21;HEADER;ENDSEC;ANCHOR;ENDSEC;DATA;ENDSEC;END-ISO-10303-21;".into(),
            |k| matches!(k, K::Unsupported { .. }),
            1,
        ),
    ];
    for (text, is_expected, line) in cases {
        let error = read(text.as_bytes()).expect_err(&text);
        assert!(is_expected(&error.kind), "{text:?}: {error:?}");
        assert_eq!(error.line, line, "{text:?}: {error}");
        assert!(error.offset <= text.len());
        assert!(error.to_string().contains(&format!("line {line} ")));
    }
    // Column and offset point at the token.
    let text = wrap("#1=A();\n#2=B(  #7);");
    let error = read(text.as_bytes()).unwrap_err();
    assert_eq!(error.kind, K::UnresolvedReference { from: 2, to: 7 });
    assert_eq!((error.line, error.column), (6, 1));
    assert_eq!(&text[error.offset..error.offset + 2], "#2");
}

#[test]
fn truncating_or_damaging_a_file_anywhere_never_panics() {
    let text = write(&sample()).unwrap();
    let bytes = text.as_bytes();
    // Every prefix that lacks more than the final line break lacks at
    // least the end of the end marker.
    for cut in 0..bytes.len() - 1 {
        assert!(read(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut random = Random(42);
    let replacements = b"()#'\"\\.,;=$*/ \nA9-!&\x00\xFF";
    for _ in 0..20_000 {
        let mut damaged = bytes.to_vec();
        for _ in 0..1 + random.next() % 3 {
            let at = (random.next() % damaged.len() as u64) as usize;
            let with = replacements[(random.next() % replacements.len() as u64) as usize];
            match random.next() % 3 {
                0 => damaged[at] = with,
                1 => damaged.insert(at, with),
                _ => {
                    damaged.remove(at);
                }
            }
        }
        // Either an error or a model; a model writes and reads back.
        if let Ok(file) = read(&damaged)
            && let Ok(text) = write(&file)
        {
            assert_eq!(read(text.as_bytes()).unwrap(), file);
        }
    }
}

#[test]
fn limits_stop_huge_counts_and_deep_nesting() {
    // Nesting: a list in a list in a list …
    let deep = |levels: usize| {
        wrap(&format!(
            "#1=A({}{});",
            "(".repeat(levels),
            ")".repeat(levels)
        ))
    };
    assert!(read(deep(60).as_bytes()).is_ok());
    for levels in [64, 65, 1000, 100_000] {
        let error = read(deep(levels).as_bytes()).unwrap_err();
        assert_eq!(
            error.kind,
            ReadErrorKind::LimitExceeded {
                what: "levels of nesting"
            },
            "{levels}"
        );
    }
    // Typed parameters nest too.
    let typed = wrap(&format!(
        "#1=A({}1{});",
        "B(".repeat(5000),
        ")".repeat(5000)
    ));
    assert!(matches!(
        read(typed.as_bytes()).unwrap_err().kind,
        ReadErrorKind::LimitExceeded { .. }
    ));
    // An unclosed run of opening parentheses must not recurse either.
    let open = wrap(&format!("#1=A({}", "(".repeat(100_000)));
    assert!(read(open.as_bytes()).is_err());

    let many: String = (1..=100).map(|i| format!("#{i}=A(1,2,(3,4));")).collect();
    let text = wrap(&many);
    assert_eq!(read(text.as_bytes()).unwrap().data.len(), 100);
    let limits = |change: fn(&mut ReadLimits)| {
        let mut limits = ReadLimits::default();
        change(&mut limits);
        read_with(text.as_bytes(), &limits).unwrap_err().kind
    };
    assert_eq!(
        limits(|l| l.max_instances = 99),
        ReadErrorKind::LimitExceeded { what: "instances" }
    );
    assert_eq!(
        limits(|l| l.max_params = 499),
        ReadErrorKind::LimitExceeded { what: "parameters" }
    );
    assert_eq!(limits(|l| l.max_bytes = 100), ReadErrorKind::TooLarge);
    let long = wrap(&format!("#1=A('{}');", "x".repeat(5000)));
    let small = ReadLimits {
        max_string: 1000,
        ..ReadLimits::default()
    };
    assert!(matches!(
        read_with(long.as_bytes(), &small).unwrap_err().kind,
        ReadErrorKind::BadString { .. }
    ));
    assert!(read(long.as_bytes()).is_ok());
}

#[test]
fn cycles_are_found_without_recursion() {
    let file = read(wrap("#1=A(#2);#2=B((#3,#4));#3=C(D(#1));#4=E();").as_bytes()).unwrap();
    assert_eq!(file.reference_cycle(), Some(vec![1, 2, 3]));
    let file = read(wrap("#1=A(#1);").as_bytes()).unwrap();
    assert_eq!(file.reference_cycle(), Some(vec![1]));
    let file = read(wrap("#1=A(#2,#3);#2=B(#4);#3=C(#4);#4=D();").as_bytes()).unwrap();
    assert_eq!(file.reference_cycle(), None, "a diamond is not a cycle");
    // A chain of 200 000 instances, then closed into a ring.
    let n = 200_000_u64;
    let mut chain = StepFile::default();
    for i in 1..n {
        chain.data.push(Instance {
            id: i,
            entity: Entity::Simple(Record::new("A", vec![Param::Ref(i + 1)])),
        });
    }
    chain.data.push(Instance {
        id: n,
        entity: Entity::Simple(Record::new("A", vec![])),
    });
    assert_eq!(chain.reference_cycle(), None);
    assert_eq!(chain.unresolved_reference(), None);
    chain.data[n as usize - 1] = Instance {
        id: n,
        entity: Entity::Simple(Record::new("A", vec![Param::Ref(1)])),
    };
    assert_eq!(chain.reference_cycle().map(|c| c.len()), Some(n as usize));
    chain.data[0] = Instance {
        id: 1,
        entity: Entity::Simple(Record::new("A", vec![Param::Ref(n + 5)])),
    };
    assert_eq!(chain.unresolved_reference(), Some((1, n + 5)));
}

#[test]
fn what_cannot_be_written_is_an_error() {
    let with = |param: Param| {
        let mut file = StepFile::default();
        file.add("A", vec![param]);
        write(&file)
    };
    assert_eq!(
        with(Param::Real(f64::NAN)),
        Err(WriteError::NonFiniteReal { instance: 1 })
    );
    assert_eq!(
        with(Param::List(vec![Param::Real(f64::INFINITY)])),
        Err(WriteError::NonFiniteReal { instance: 1 })
    );
    assert!(matches!(
        with(Param::enumeration("t")),
        Err(WriteError::BadEnum { .. })
    ));
    assert!(matches!(
        with(Param::enumeration("")),
        Err(WriteError::BadEnum { .. })
    ));
    assert!(matches!(
        with(Param::Binary("XYZ".into())),
        Err(WriteError::BadBinary { .. })
    ));
    assert!(matches!(
        with(Param::typed("lower", Param::Integer(1))),
        Err(WriteError::BadKeyword { .. })
    ));
    assert_eq!(with(Param::Ref(0)), Err(WriteError::BadId { instance: 1 }));
    let mut nested = Param::Integer(1);
    for _ in 0..70 {
        nested = Param::List(vec![nested]);
    }
    assert_eq!(with(nested), Err(WriteError::TooDeep { instance: 1 }));

    let mut file = StepFile::default();
    file.add("A B", vec![]);
    assert!(matches!(write(&file), Err(WriteError::BadKeyword { .. })));
    let mut file = StepFile::default();
    file.add_complex(vec![]);
    assert_eq!(write(&file), Err(WriteError::EmptyComplex { instance: 1 }));
    let mut file = StepFile::default();
    file.add("A", vec![]);
    file.data.push(file.data[0].clone());
    assert_eq!(write(&file), Err(WriteError::BadId { instance: 1 }));
    file.data[1].id = 0;
    assert_eq!(write(&file), Err(WriteError::BadId { instance: 0 }));
    let mut file = StepFile::default();
    file.header.push(Record::new("x", vec![]));
    let error = write(&file).unwrap_err();
    assert!(error.to_string().contains("the header"), "{error}");
}

#[test]
fn ids_count_up_in_the_order_of_adding() {
    let mut file = StepFile::default();
    assert_eq!(file.add("A", vec![]), 1);
    assert_eq!(file.add_complex(vec![Record::new("B", vec![])]), 2);
    assert_eq!(file.add("C", vec![]), 3);
    // After a file with gaps was read, adding continues past the largest.
    let mut read_back = read(wrap("#7=A();#3=B();").as_bytes()).unwrap();
    assert_eq!(read_back.add("C", vec![]), 8);
    assert_eq!(read_back.add("D", vec![]), 9);
    // Long lists wrap at commas and still read back.
    let mut long = StepFile::default();
    long.add(
        "A",
        vec![Param::reals((0..500).map(|i| f64::from(i) * 0.37))],
    );
    let text = write(&long).unwrap();
    assert!(text.lines().all(|l| l.len() <= 100), "{text}");
    assert_eq!(read(text.as_bytes()).unwrap(), long);
}
