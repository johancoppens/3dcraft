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
        ZipError::Method {
            name: "a".into(),
            method: 12,
        },
        ZipError::Inflate {
            name: "a".into(),
            error: InflateError::Truncated,
        },
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

// Deflated archives (`read`), from Python's zipfile and zlib
// (testdata/, made by a few lines of Python: a compressible text, a short
// text, 3000 random bytes, a stored entry and an empty one).
const DEFLATED: &[u8] = include_bytes!("../testdata/deflated.zip");
const TEXT: &[u8] = include_bytes!("../testdata/deflated.text");
const NOISE: &[u8] = include_bytes!("../testdata/noise.bin");
const LEVEL0: &[u8] = include_bytes!("../testdata/level0.deflate");

#[test]
fn deflated_entries_read_back_exactly() {
    let entries = read(DEFLATED).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
    assert_eq!(
        names,
        [
            "Document.xml",
            "short.txt",
            "noise.bin",
            "stored.txt",
            "empty"
        ]
    );
    let data: Vec<Vec<u8>> = entries
        .iter()
        .map(|e| e.data().unwrap().into_owned())
        .collect();
    assert_eq!(data[0], TEXT);
    assert_eq!(data[1], b"Hello, Document.xml!");
    assert_eq!(data[2], NOISE);
    assert_eq!(data[3], b"stored as is");
    assert!(data[4].is_empty());
    assert_eq!(entries[0].method, 8);
    assert_eq!(entries[3].method, 0);
    // The stored-only reader still refuses them, saying so.
    assert!(matches!(
        read_stored(DEFLATED),
        Err(ZipError::Compressed { method: 8, .. })
    ));
}

#[test]
fn stored_deflate_blocks_and_the_limit() {
    assert_eq!(inflate::inflate(LEVEL0, TEXT.len()).unwrap(), TEXT);
    assert_eq!(
        inflate::inflate(LEVEL0, TEXT.len() - 1),
        Err(InflateError::TooLong)
    );
    // Reserved block type 3.
    assert!(matches!(
        inflate::inflate(&[0x07], 10),
        Err(InflateError::Invalid(_))
    ));
    assert_eq!(inflate::inflate(&[], 10), Err(InflateError::Truncated));
}

#[test]
fn a_small_stream_cannot_expand_past_its_limit() {
    // A megabyte of zeros in about a kilobyte (zlib, level 9).
    let zeros: &[u8] = include_bytes!("../testdata/zeros.deflate");
    assert!(zeros.len() < 2048);
    assert_eq!(inflate::inflate(zeros, 1000), Err(InflateError::TooLong));
    let all = inflate::inflate(zeros, 1 << 20).unwrap();
    assert_eq!(all.len(), 1 << 20);
    assert!(all.iter().all(|&b| b == 0));
}

#[test]
fn damaged_deflated_archives_never_panic() {
    // Every byte flipped, and every cut: an error or the right data.
    for at in 0..DEFLATED.len() {
        let mut bytes = DEFLATED.to_vec();
        bytes[at] ^= 0x5a;
        if let Ok(entries) = read(&bytes) {
            for e in entries {
                if let Ok(data) = e.data() {
                    let expected: &[u8] = match e.name {
                        "Document.xml" => TEXT,
                        "noise.bin" => NOISE,
                        "short.txt" => b"Hello, Document.xml!",
                        "stored.txt" => b"stored as is",
                        _ => &[],
                    };
                    // A changed name or size may make another entry; data
                    // that comes back under a known name is that data.
                    if !expected.is_empty() && data.len() == expected.len() {
                        assert_eq!(&data[..], expected, "flip at {at}");
                    }
                }
            }
        }
        let _ = read(&DEFLATED[..at]);
    }
    for at in 0..LEVEL0.len().min(400) {
        let mut bytes = LEVEL0.to_vec();
        bytes[at] ^= 0xff;
        let _ = inflate::inflate(&bytes, TEXT.len());
        let _ = inflate::inflate(&LEVEL0[..at], TEXT.len());
    }
}

// Raw streams from zlib (level 9): a dynamic-Huffman block and a
// fixed-Huffman block of the same text.
const DYNAMIC: &[u8] = include_bytes!("../testdata/dynamic.deflate");
const FIXED: &[u8] = include_bytes!("../testdata/fixed.deflate");

#[test]
fn dynamic_and_fixed_huffman_blocks_read_back_exactly() {
    assert_eq!(DYNAMIC[0] & 7, 0b101, "a last block of the dynamic kind");
    assert_eq!(FIXED[0] & 7, 0b011, "a last block of the fixed kind");
    assert_eq!(inflate::inflate(DYNAMIC, TEXT.len()).unwrap(), TEXT);
    assert_eq!(inflate::inflate(FIXED, TEXT.len()).unwrap(), TEXT);
}

#[test]
fn damaged_huffman_streams_never_panic() {
    // Every byte of both streams changed three ways, and every cut: an
    // error, or (rarely) some bytes within the limit.
    for stream in [DYNAMIC, FIXED] {
        for at in 0..stream.len() {
            for delta in [1u8, 0x10, 0x80] {
                let mut bytes = stream.to_vec();
                bytes[at] = bytes[at].wrapping_add(delta);
                if let Ok(out) = inflate::inflate(&bytes, TEXT.len()) {
                    assert!(out.len() <= TEXT.len());
                }
            }
            let _ = inflate::inflate(&stream[..at], TEXT.len());
        }
    }
}

// `write`: Unix modes and a deflater the caller hands in (the MCP bundle of
// the release, xtask/src/mcpb.rs).

/// Entries of several sizes (none, one byte, sizes on both sides of 255
/// and 256 and past 65535, so every byte of the size and offset fields
/// takes part) under names in folders with a non-ASCII character.
fn golden_sample() -> Vec<(String, Vec<u8>)> {
    [0_usize, 1, 5, 255, 256, 1000, 70_000]
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let name = format!("dir{}/entry-{i}-\u{e9}.bin", i % 3);
            let data: Vec<u8> = (0..n).map(|k| ((k * 31 + i * 7) % 251) as u8).collect();
            (name, data)
        })
        .collect()
}

/// SHA-256 (FIPS 180-4) of `data` in lower-case hex: how these tests pin
/// a whole archive.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0_u32; 64];
        for (word, bytes) in w.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (k, word) in K.iter().zip(w) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(*k)
                .wrapping_add(word);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (word, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *word = word.wrapping_add(add);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}

/// What `write_stored` wrote for the golden sample before `write` was
/// added (2026-11-20), pinned by its length and SHA-256: the values were
/// printed by this sample on the base, `release/0.2` at f6d88a04 (the
/// workshop's Check run 44718142541), and agree with an independent
/// writer of the documented layout (a script, field for field). `.3dcraft`
/// files and 3MF packages are written by `write_stored`, so their bytes
/// stay what released versions wrote (determinism, AGENTS.md rule 5);
/// `write` without modes or a deflater writes the same bytes.
#[test]
fn write_stored_writes_the_bytes_it_wrote_before_write_was_added() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let thousand: Vec<u8> = (0..1000_usize).map(|k| ((k * 7) % 256) as u8).collect();
    assert_eq!(
        sha256_hex(&thousand),
        "89f4ff56a25dd1db06a4ce6033603775d705fb96f30f8693733fef602a1ca532"
    );
    let sample = golden_sample();
    let entries: Vec<(&str, &[u8])> = sample
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    let new: Vec<NewEntry<'_>> = entries
        .iter()
        .map(|(name, data)| NewEntry::new(name, data))
        .collect();
    for archive in [write_stored(&entries).unwrap(), write(&new, None).unwrap()] {
        assert_eq!(archive.len(), 72_337);
        assert_eq!(
            sha256_hex(&archive),
            "e2fc746366ddbef47e5d2e2441cbed64d76c810a7a4079b93f4b943e884af317"
        );
    }
}

/// The one entry of `the_output_is_exactly_this` with the mode 0o755: the
/// central directory says the archive was made on Unix (host 3) and holds
/// a regular file's mode (0o100755) in the high half of the external
/// attributes, as Info-ZIP's zip writes them; nothing else changes.
#[test]
fn a_unix_mode_is_recorded_as_unix_tools_record_it() {
    let entry = NewEntry {
        unix_mode: Some(0o755),
        ..NewEntry::new("a", b"hi")
    };
    let bytes = write(&[entry], None).unwrap();
    let (_, central) = layout(&bytes);
    let mut expected = write_stored(&[("a", b"hi")]).unwrap();
    expected[central + 4..central + 6].copy_from_slice(&[20, 3]);
    expected[central + 38..central + 42].copy_from_slice(&[0, 0, 0xED, 0x81]);
    assert_eq!(bytes, expected);
    assert_eq!(u32_at(&bytes, central + 38), Some(0o100_755 << 16));
}

#[test]
fn modes_read_back_and_an_entry_without_one_has_none() {
    let entries = [
        NewEntry {
            unix_mode: Some(0o755),
            ..NewEntry::new("bin/program", b"\x7fELF")
        },
        NewEntry {
            unix_mode: Some(0o644),
            ..NewEntry::new("data.txt", b"text")
        },
        NewEntry::new("plain", b"x"),
        // Only the permission bits are kept: a file type given in the mode
        // is replaced by a regular file's.
        NewEntry {
            unix_mode: Some(0o170_640),
            ..NewEntry::new("typed", b"y")
        },
    ];
    let bytes = write(&entries, None).unwrap();
    let back = read(&bytes).unwrap();
    let modes: Vec<Option<u16>> = back.iter().map(|e| e.unix_mode).collect();
    assert_eq!(modes, [Some(0o755), Some(0o644), None, Some(0o640)]);
    for (entry, got) in entries.iter().zip(&back) {
        assert_eq!(
            (entry.name, entry.data),
            (got.name, &got.data().unwrap()[..])
        );
        assert_eq!(got.method, 0);
    }
    // The stored-only reader reads them too: a mode changes no data.
    assert_eq!(read_stored(&bytes).unwrap().len(), 4);
    // Archives without modes read back without them; one another program
    // wrote on Unix, with its modes (Python's zipfile records 0o600 for
    // what `writestr` adds: testdata/deflated.zip).
    assert!(
        read(&sample())
            .unwrap()
            .iter()
            .all(|e| e.unix_mode.is_none())
    );
    assert!(
        read(DEFLATED)
            .unwrap()
            .iter()
            .all(|e| e.unix_mode == Some(0o600))
    );
}

/// The deflater decides: an entry it compresses is written deflated with
/// its stream as it is, and reads back exactly; one it leaves alone is
/// stored. `DYNAMIC` is zlib's raw deflate of `TEXT` (testdata/).
#[test]
fn a_deflater_writes_deflated_entries_that_read_back_exactly() {
    let deflate = |data: &[u8]| (data == TEXT).then(|| DYNAMIC.to_vec());
    let entries = [
        NewEntry {
            unix_mode: Some(0o644),
            ..NewEntry::new("Document.xml", TEXT)
        },
        NewEntry::new("short.txt", b"left as it is"),
        NewEntry::new("empty", b""),
    ];
    let bytes = write(&entries, Some(&deflate)).unwrap();
    let back = read(&bytes).unwrap();
    assert_eq!(back[0].method, 8);
    assert_eq!(back[0].size, TEXT.len());
    assert_eq!(back[0].unix_mode, Some(0o644));
    assert_eq!(&back[0].data().unwrap()[..], TEXT);
    assert_eq!(back[1].method, 0);
    assert_eq!(&back[1].data().unwrap()[..], b"left as it is");
    assert!(back[2].data().unwrap().is_empty());
    // The deflated entry holds the deflater's stream, byte for byte, and
    // the archive is that much smaller than its stored form.
    assert!(bytes.windows(DYNAMIC.len()).any(|window| window == DYNAMIC));
    let stored = write(&entries, None).unwrap();
    assert_eq!(stored.len() - bytes.len(), TEXT.len() - DYNAMIC.len());
    assert_eq!(
        write(&entries, Some(&deflate)).unwrap(),
        bytes,
        "deterministic"
    );
    // The stored-only reader refuses it, saying it is compressed.
    assert!(matches!(
        read_stored(&bytes),
        Err(ZipError::Compressed { method: 8, .. })
    ));
}

/// A deflater that returns a stream which is not the data's: the archive
/// is written (the CRC is the data's), and reading the entry says it is
/// damaged rather than returning other data.
#[test]
fn a_wrong_deflate_stream_is_found_when_read() {
    let wrong = |_: &[u8]| Some(DYNAMIC.to_vec());
    let bytes = write(&[NewEntry::new("a.txt", b"not the text")], Some(&wrong)).unwrap();
    let back = read(&bytes).unwrap();
    assert!(back[0].data().is_err());
}

#[test]
fn write_refuses_what_write_stored_refuses() {
    for name in BAD_NAMES {
        let entry = NewEntry {
            unix_mode: Some(0o755),
            ..NewEntry::new(name, b"x")
        };
        assert!(
            matches!(write(&[entry], None), Err(ZipError::InvalidName { .. })),
            "`{name}`"
        );
    }
    let twice = [NewEntry::new("a", b"1"), NewEntry::new("a", b"2")];
    assert_eq!(
        write(&twice, None),
        Err(ZipError::DuplicateName("a".into()))
    );
}
