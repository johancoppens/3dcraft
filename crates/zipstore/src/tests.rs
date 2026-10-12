use super::*;

/// The three entries most tests use: plain, non-ASCII name with binary
/// data, and empty.
const SAMPLE: [(&str, &[u8]); 3] = [
    ("a.txt", b"hello"),
    ("dir/\u{fc}.bin", &[0, 255, 7]),
    ("empty", b""),
];

fn sample() -> Vec<u8> {
    write_stored(&SAMPLE).unwrap()
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Offsets of the end record and of the first central directory entry, for
/// an archive without a comment.
fn layout(bytes: &[u8]) -> (usize, usize) {
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    (end, u32_at(bytes, end + 16).unwrap() as usize)
}

/// One entry of [`raw`]: everything the writer would compute can be set to
/// something wrong.
#[derive(Clone)]
struct Raw {
    name: Vec<u8>,
    data: Vec<u8>,
    flags: u16,
    method: u16,
    /// `None` computes the right one.
    crc: Option<u32>,
}

impl Raw {
    fn new(name: &[u8], data: &[u8]) -> Self {
        Self {
            name: name.to_vec(),
            data: data.to_vec(),
            flags: 1 << 11,
            method: 0,
            crc: None,
        }
    }
}

/// An archive written by hand, with no checks at all, so tests can build
/// what [`write_stored`] refuses to.
fn raw(entries: &[Raw]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let offset = out.len() as u32;
        let mut common = Vec::new();
        common.extend_from_slice(&20_u16.to_le_bytes());
        common.extend_from_slice(&e.flags.to_le_bytes());
        common.extend_from_slice(&e.method.to_le_bytes());
        common.extend_from_slice(&[0, 0, 33, 0]); // time, date
        common.extend_from_slice(&e.crc.unwrap_or_else(|| crc32(&e.data)).to_le_bytes());
        common.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        common.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        common.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        common.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&common);
        out.extend_from_slice(&e.name);
        out.extend_from_slice(&e.data);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&common);
        central.extend_from_slice(&[0; 10]); // comment, disk, attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(&e.name);
    }
    end_record(&mut out, &central, entries.len() as u16);
    out
}

/// Appends a central directory and its end record.
fn end_record(out: &mut Vec<u8>, central: &[u8], count: u16) {
    let central_offset = out.len() as u32;
    out.extend_from_slice(central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
}

fn one(name: &[u8]) -> Vec<u8> {
    raw(&[Raw::new(name, b"data")])
}

#[test]
fn crc32_matches_the_standard_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn entries_round_trip_including_an_empty_one_and_a_non_ascii_name() {
    let bytes = sample();
    let back = read_stored(&bytes).unwrap();
    assert_eq!(back.len(), 3);
    for ((name, data), entry) in SAMPLE.iter().zip(&back) {
        assert_eq!((*name, *data), (entry.name, entry.data));
    }
    assert_eq!(read_stored(&write_stored(&[]).unwrap()).unwrap(), []);
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    assert_eq!(sample(), sample());
}

/// The whole layout, byte for byte: 3MF and `.3dcraft` files must not
/// change when this crate is touched.
#[test]
fn the_output_is_exactly_this() {
    #[rustfmt::skip]
    let expected: &[u8] = &[
        // Local file header.
        b'P', b'K', 3, 4,
        20, 0,                  // version needed: 2.0
        0, 8,                   // flags: UTF-8 names
        0, 0,                   // method: stored
        0, 0, 33, 0,            // time 00:00, date 1980-01-01
        0xAC, 0x2A, 0x93, 0xD8, // CRC-32 of "hi"
        2, 0, 0, 0,             // compressed size
        2, 0, 0, 0,             // uncompressed size
        1, 0,                   // name length
        0, 0,                   // extra length
        b'a',
        b'h', b'i',
        // Central directory entry.
        b'P', b'K', 1, 2,
        20, 0,                  // version made by
        20, 0, 0, 8, 0, 0, 0, 0, 33, 0,
        0xAC, 0x2A, 0x93, 0xD8, 2, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0,
        0, 0,                   // comment length
        0, 0,                   // disk number
        0, 0,                   // internal attributes
        0, 0, 0, 0,             // external attributes
        0, 0, 0, 0,             // offset of the local header
        b'a',
        // End of central directory.
        b'P', b'K', 5, 6,
        0, 0, 0, 0,             // disk numbers
        1, 0, 1, 0,             // entries on this disk, in total
        47, 0, 0, 0,            // central directory size
        33, 0, 0, 0,            // central directory offset
        0, 0,                   // comment length
    ];
    assert_eq!(write_stored(&[("a", b"hi")]).unwrap(), expected);
    // The hand-written builder of these tests agrees with the writer.
    assert_eq!(raw(&[Raw::new(b"a", b"hi")]), expected);
}

#[test]
fn a_trailing_comment_is_allowed() {
    let mut bytes = sample();
    let (end, _) = layout(&bytes);
    let comment = b"written by a test; PK\x05\x06 inside it";
    put16(&mut bytes, end + 20, comment.len() as u16);
    bytes.extend_from_slice(comment);
    assert_eq!(read_stored(&bytes).unwrap().len(), 3);
    // The longest comment there can be.
    let mut bytes = sample();
    put16(&mut bytes, end + 20, u16::MAX);
    bytes.resize(bytes.len() + 65535, b' ');
    assert_eq!(read_stored(&bytes).unwrap().len(), 3);
    // A comment that is shorter or longer than the record says.
    bytes.pop();
    assert_eq!(read_stored(&bytes), Err(ZipError::NoEndRecord));
    bytes.extend_from_slice(b"  ");
    assert_eq!(read_stored(&bytes), Err(ZipError::NoEndRecord));
}

#[test]
fn an_archive_cut_short_anywhere_is_an_error() {
    let bytes = sample();
    for len in 0..bytes.len() {
        assert!(read_stored(&bytes[..len]).is_err(), "cut at {len}");
    }
    assert_eq!(read_stored(b""), Err(ZipError::NoEndRecord));
    assert_eq!(read_stored(b"not a zip"), Err(ZipError::NoEndRecord));
    // Cut at the front instead: every offset is now wrong.
    for start in 1..bytes.len() {
        assert!(read_stored(&bytes[start..]).is_err(), "from {start}");
    }
}

/// Damage to one byte is either caught or harmless: whatever the reader
/// returns is exactly what was stored, CRC included.
#[test]
fn flipping_any_byte_never_panics_and_never_returns_wrong_data() {
    let bytes = sample();
    let mut accepted = 0;
    for at in 0..bytes.len() {
        for mask in [0x01, 0x10, 0x80, 0xFF] {
            let mut damaged = bytes.clone();
            damaged[at] ^= mask;
            if let Ok(entries) = read_stored(&damaged) {
                accepted += 1;
                assert_eq!(entries.len(), SAMPLE.len(), "byte {at} ^ {mask:#x}");
                for ((name, data), entry) in SAMPLE.iter().zip(&entries) {
                    assert_eq!((*name, *data), (entry.name, entry.data), "byte {at}");
                    assert_eq!(crc32(entry.data), crc32(data));
                }
            }
        }
    }
    // Some fields (timestamps, attributes) carry no meaning for the reader,
    // so the sweep does reach the Ok branch.
    assert!(accepted > 0);
}

#[test]
fn a_compressed_entry_is_refused_with_a_message_that_says_so() {
    let mut entry = Raw::new(b"a", b"data");
    entry.method = 8;
    let error = read_stored(&raw(&[entry])).unwrap_err();
    assert_eq!(
        error,
        ZipError::Compressed {
            name: "a".into(),
            method: 8
        }
    );
    assert!(
        error.to_string().contains("compression is not supported"),
        "{error}"
    );
    // The method in the local header alone.
    let mut bytes = one(b"a");
    put16(&mut bytes, 8, 8);
    assert!(matches!(
        read_stored(&bytes),
        Err(ZipError::Compressed { .. })
    ));
}

#[test]
fn an_encrypted_entry_is_refused() {
    for bit in [0, 6] {
        let mut entry = Raw::new(b"secret", b"data");
        entry.flags |= 1 << bit;
        assert_eq!(
            read_stored(&raw(&[entry])),
            Err(ZipError::Encrypted {
                name: "secret".into()
            })
        );
    }
    // The flag in the local header alone.
    let mut bytes = one(b"secret");
    put16(&mut bytes, 6, 1);
    assert!(matches!(
        read_stored(&bytes),
        Err(ZipError::Encrypted { .. })
    ));
}

#[test]
fn zip64_markers_are_refused() {
    let valid = one(b"a");
    let (end, central) = layout(&valid);
    // In the end record: entry counts, directory size, directory offset.
    for (at, width) in [(end + 8, 2), (end + 10, 2), (end + 12, 4), (end + 16, 4)] {
        let mut bytes = valid.clone();
        bytes[at..at + width].fill(0xFF);
        assert_eq!(read_stored(&bytes), Err(ZipError::Zip64), "field at {at}");
    }
    // In a central directory entry: sizes and local header offset.
    for at in [central + 20, central + 24, central + 42] {
        let mut bytes = valid.clone();
        put32(&mut bytes, at, u32::MAX);
        assert_eq!(read_stored(&bytes), Err(ZipError::Zip64), "field at {at}");
    }
    // A ZIP64 locator in front of the end record.
    let mut bytes = valid.clone();
    let mut locator = b"PK\x06\x07".to_vec();
    locator.resize(20, 0);
    bytes.splice(end..end, locator);
    assert_eq!(read_stored(&bytes), Err(ZipError::Zip64));
}

#[test]
fn multi_disk_archives_are_refused() {
    let valid = one(b"a");
    let (end, central) = layout(&valid);
    for at in [end + 4, end + 6, central + 34] {
        let mut bytes = valid.clone();
        put16(&mut bytes, at, 1);
        assert_eq!(read_stored(&bytes), Err(ZipError::MultiDisk), "at {at}");
    }
}

#[test]
fn a_crc_mismatch_is_refused() {
    // Damaged data.
    let mut bytes = one(b"a");
    bytes[31] ^= 1;
    let error = read_stored(&bytes).unwrap_err();
    assert_eq!(
        error,
        ZipError::CrcMismatch {
            name: "a".into(),
            expected: crc32(b"data"),
            actual: crc32(b"eata"),
        }
    );
    assert!(error.to_string().contains("CRC-32"), "{error}");
    // A wrong CRC written consistently in both headers.
    let mut entry = Raw::new(b"a", b"data");
    entry.crc = Some(0);
    assert!(matches!(
        read_stored(&raw(&[entry])),
        Err(ZipError::CrcMismatch { expected: 0, .. })
    ));
}

#[test]
fn a_name_that_is_not_utf8_is_refused() {
    assert_eq!(read_stored(&one(b"caf\xE9")), Err(ZipError::NameNotUtf8));
    assert_eq!(read_stored(&one(b"\xFF")), Err(ZipError::NameNotUtf8));
}

#[test]
fn duplicate_names_are_refused() {
    let bytes = raw(&[
        Raw::new(b"a", b"1"),
        Raw::new(b"b", b"2"),
        Raw::new(b"a", b"3"),
    ]);
    assert_eq!(
        read_stored(&bytes),
        Err(ZipError::DuplicateName("a".into()))
    );
}

const BAD_NAMES: [&str; 12] = [
    "",
    "/etc/passwd",
    "/",
    "..",
    "../a",
    "a/../b",
    "a/..",
    "a\\b",
    "..\\a",
    "C:/a",
    "c:a",
    "a\0b",
];

#[test]
fn absolute_names_parent_components_and_backslashes_are_refused() {
    for name in BAD_NAMES {
        assert!(
            matches!(
                read_stored(&one(name.as_bytes())),
                Err(ZipError::InvalidName { name: n, .. }) if n == name
            ),
            "`{name}`"
        );
    }
    // Dots that are not a `..` component are ordinary names.
    for name in [
        "..a",
        "a..",
        "a/...",
        ".hidden",
        "a/./b",
        "dir/",
        "3D/x.model",
    ] {
        let bytes = one(name.as_bytes());
        assert_eq!(read_stored(&bytes).unwrap()[0].name, name, "`{name}`");
    }
}

#[test]
fn more_entries_than_the_cap_are_refused() {
    let entries = |count: usize| -> Vec<Raw> {
        (0..count)
            .map(|i| Raw::new(format!("e{i}").as_bytes(), b""))
            .collect()
    };
    assert_eq!(
        read_stored(&raw(&entries(MAX_ENTRIES))).unwrap().len(),
        MAX_ENTRIES
    );
    assert_eq!(
        read_stored(&raw(&entries(MAX_ENTRIES + 1))),
        Err(ZipError::TooManyEntries {
            count: MAX_ENTRIES + 1
        })
    );
    // A count without the entries to back it costs nothing.
    let mut bytes = one(b"a");
    let (end, _) = layout(&bytes);
    put16(&mut bytes, end + 8, 4000);
    put16(&mut bytes, end + 10, 4000);
    assert!(matches!(read_stored(&bytes), Err(ZipError::Corrupt(_))));
}

#[test]
fn offsets_and_sizes_outside_the_file_are_refused() {
    let valid = one(b"a");
    let (end, central) = layout(&valid);
    let damaged = |at: usize, value: u32| {
        let mut bytes = valid.clone();
        put32(&mut bytes, at, value);
        read_stored(&bytes).map(|_| ())
    };
    let len = valid.len() as u32;
    // The central directory: offset or size past the end, or overflowing.
    for (at, value) in [
        (end + 16, len),
        (end + 16, u32::MAX - 1),
        (end + 12, len),
        (end + 12, u32::MAX - 1),
    ] {
        assert!(
            matches!(damaged(at, value), Err(ZipError::OutOfBounds(_))),
            "{at} = {value}: {:?}",
            damaged(at, value)
        );
    }
    // Inside the file, but not where the directory is.
    assert!(matches!(damaged(end + 16, 0), Err(ZipError::Corrupt(_))));
    assert!(matches!(damaged(end + 12, 0), Err(ZipError::Corrupt(_))));
    // The local header of an entry.
    for value in [len, len - 10, u32::MAX - 1] {
        assert_eq!(
            damaged(central + 42, value),
            Err(ZipError::OutOfBounds("a local file header")),
            "{value}"
        );
    }
    // The data of an entry: a size (in all four places) that runs into the
    // central directory, past the end of the file, or near overflow.
    for size in [5, len, u32::MAX - 1] {
        let mut bytes = valid.clone();
        for at in [18, 22, central + 20, central + 24] {
            put32(&mut bytes, at, size);
        }
        assert_eq!(
            read_stored(&bytes),
            Err(ZipError::OutOfBounds("the data of an entry")),
            "{size}"
        );
    }
    // Name and extra lengths that run out of the file or the directory.
    for at in [26, 28, central + 28, central + 30, central + 32] {
        let mut bytes = valid.clone();
        put16(&mut bytes, at, 0xFFF0);
        assert!(read_stored(&bytes).is_err(), "length at {at}");
    }
}

#[test]
fn headers_that_contradict_each_other_are_refused() {
    let valid = one(b"ab");
    let (_, central) = layout(&valid);
    let mismatch = Err(ZipError::HeaderMismatch { name: "ab".into() });
    // The name in the local header.
    let mut bytes = valid.clone();
    bytes[30] = b'x';
    assert_eq!(read_stored(&bytes), mismatch);
    // CRC and size in the local header.
    for at in [14, 18, 22] {
        let mut bytes = valid.clone();
        bytes[at] ^= 1;
        assert_eq!(read_stored(&bytes), mismatch, "local field at {at}");
    }
    // Different sizes for a stored entry.
    let mut bytes = valid.clone();
    put32(&mut bytes, central + 20, 3);
    assert!(matches!(read_stored(&bytes), Err(ZipError::Corrupt(_))));
    // Wrong signatures.
    for at in [0, central] {
        let mut bytes = valid.clone();
        bytes[at + 2] = 9;
        assert!(
            matches!(read_stored(&bytes), Err(ZipError::Corrupt(_))),
            "{at}"
        );
    }
    // More directory than entries.
    let mut bytes = raw(&[Raw::new(b"a", b""), Raw::new(b"b", b"")]);
    let (end, _) = layout(&bytes);
    put16(&mut bytes, end + 8, 1);
    put16(&mut bytes, end + 10, 1);
    assert!(matches!(read_stored(&bytes), Err(ZipError::Corrupt(_))));
}

/// Archives written in one pass leave CRC and sizes out of the local header
/// (general-purpose bit 3); the central directory has them.
#[test]
fn a_data_descriptor_entry_is_read_from_the_central_directory() {
    let mut bytes = one(b"a");
    put16(&mut bytes, 6, (1 << 11) | (1 << 3));
    bytes[14..26].fill(0);
    let entries = read_stored(&bytes).unwrap();
    assert_eq!((entries[0].name, entries[0].data), ("a", &b"data"[..]));
}

/// Entries nested in each other's data (each local header sits inside the
/// data of the previous entry) would make the reader checksum the same
/// bytes once per entry. The total is capped at the file size.
#[test]
fn entries_that_overlap_cannot_multiply_the_work() {
    let levels = 60;
    let mut blob = vec![7_u8; 2000];
    let mut locals = Vec::new();
    for level in (0..levels).rev() {
        let entry = Raw::new(format!("n{level}").as_bytes(), &blob);
        let archive = raw(&[entry]);
        let (_, central) = layout(&archive);
        locals.push((archive[central..archive.len() - 22].to_vec(), blob.len()));
        blob = archive[..central].to_vec();
    }
    // `blob` is the outermost local header with everything nested in it.
    // Each directory entry points at its own header inside it.
    let mut out = blob;
    let mut central = Vec::new();
    for (mut entry, data_len) in locals.into_iter().rev() {
        let name_len = entry.len() - 46;
        let offset = out.len() - data_len - 30 - name_len;
        put32(&mut entry, 42, offset as u32);
        central.extend_from_slice(&entry);
    }
    end_record(&mut out, &central, levels as u16);
    assert_eq!(read_stored(&out), Err(ZipError::Corrupt("entries overlap")));
    assert!(out.len() < 10_000, "and the file is small: {}", out.len());
}

#[test]
fn the_writer_refuses_what_the_reader_would_refuse() {
    for name in BAD_NAMES {
        assert!(
            matches!(
                write_stored(&[(name, b"x")]),
                Err(ZipError::InvalidName { .. })
            ),
            "`{name}`"
        );
    }
    assert_eq!(
        write_stored(&[("a", b"1"), ("a", b"2")]),
        Err(ZipError::DuplicateName("a".into()))
    );
    let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("e{i}")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
    assert_eq!(
        write_stored(&entries),
        Err(ZipError::TooManyEntries {
            count: MAX_ENTRIES + 1
        })
    );
    assert_eq!(
        read_stored(&write_stored(&entries[..MAX_ENTRIES]).unwrap())
            .unwrap()
            .len(),
        MAX_ENTRIES
    );
    let long = "n".repeat(usize::from(u16::MAX) + 1);
    assert_eq!(write_stored(&[(&long, b"")]), Err(ZipError::TooLarge));
}

#[test]
fn every_error_has_a_message() {
    let errors = [
        ZipError::TooLarge,
        ZipError::TooManyEntries { count: 5000 },
        ZipError::NoEndRecord,
        ZipError::Zip64,
        ZipError::MultiDisk,
        ZipError::Corrupt("entries overlap"),
        ZipError::OutOfBounds("the central directory"),
        ZipError::HeaderMismatch { name: "a".into() },
        ZipError::NameNotUtf8,
        ZipError::DuplicateName("a".into()),
    ];
    for error in errors {
        let text = error.to_string();
        assert!(text.len() > 20 && !text.contains("ZipError"), "{text}");
    }
    assert!(
        ZipError::TooManyEntries { count: 5000 }
            .to_string()
            .contains("5000")
    );
}
