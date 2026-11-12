//! What a Windows program asks of the system before it runs: the DLLs it
//! imports, those it loads on first use (delay-load), the machine it is
//! for and its subsystem (a console program or one with windows only),
//! read from its headers as the Microsoft PE/COFF specification lays them
//! out (the DOS header's `e_lfanew`, the `PE\0\0` signature, the COFF file
//! header, the optional header with its data directories, the section
//! table, the import and delay-load import directory tables).
//!
//! `cargo xtask package` reads the programs it packs for Windows with it
//! and refuses one that needs a runtime Windows does not bring
//! (`crate::package`). The file is input: every offset is checked against
//! it, every table has a bound, and a file that is not such a program,
//! cut short or malformed, is an `Err`, never a panic.

/// What [`read`] found in a Windows program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The COFF `Machine`: 0x8664 x86-64, 0xAA64 ARM64, 0x14C x86.
    pub machine: u16,
    /// The optional header's `Subsystem`: [`GUI`] or [`CONSOLE`] for a
    /// program.
    pub subsystem: u16,
    /// The DLLs the loader loads with the program, in the order of its
    /// import directory table.
    pub imports: Vec<String>,
    /// The DLLs it loads on first use, in the order of its delay-load
    /// directory table.
    pub delayed: Vec<String>,
}

/// `IMAGE_SUBSYSTEM_WINDOWS_GUI`: a program with windows only; started
/// from Explorer or the Start menu it gets no console.
pub const GUI: u16 = 2;
/// `IMAGE_SUBSYSTEM_WINDOWS_CUI`: a console program; started from Explorer
/// or the Start menu it gets a console window of its own.
pub const CONSOLE: u16 = 3;

/// How many entries an import or delay-load table may have before the
/// file is taken for malformed (a real program imports a few dozen DLLs).
const ENTRIES_MOST: usize = 4096;

/// The longest DLL name read (Windows' `MAX_PATH`).
const NAME_MOST: usize = 260;

/// The data directories read: the import table and the delay-load import
/// table.
const IMPORT_DIRECTORY: usize = 1;
const DELAY_IMPORT_DIRECTORY: usize = 13;

/// The magic of the optional header of a 32-bit and of a 64-bit image.
const PE32: u16 = 0x10b;
const PE32_PLUS: u16 = 0x20b;

/// The name of a machine, for the log.
pub fn machine_name(machine: u16) -> String {
    match machine {
        0x8664 => "x86-64".to_string(),
        0xaa64 => "ARM64".to_string(),
        0x014c => "x86".to_string(),
        other => format!("machine {other:#06x}"),
    }
}

/// The name of a subsystem, for the log.
pub fn subsystem_name(subsystem: u16) -> String {
    match subsystem {
        GUI => "Windows GUI subsystem".to_string(),
        CONSOLE => "console subsystem".to_string(),
        other => format!("subsystem {other}"),
    }
}

/// `len` bytes of `file` at `at`.
fn bytes(file: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    at.checked_add(len)
        .and_then(|end| file.get(at..end))
        .ok_or_else(|| {
            format!(
                "cut short or malformed: {len} bytes at offset {at:#x} lie beyond its end ({} bytes)",
                file.len()
            )
        })
}

fn u16_at(file: &[u8], at: usize) -> Result<u16, String> {
    let b: [u8; 2] = bytes(file, at, 2)?
        .try_into()
        .map_err(|_| "a field of the wrong size".to_string())?;
    Ok(u16::from_le_bytes(b))
}

fn u32_at(file: &[u8], at: usize) -> Result<u32, String> {
    let b: [u8; 4] = bytes(file, at, 4)?
        .try_into()
        .map_err(|_| "a field of the wrong size".to_string())?;
    Ok(u32::from_le_bytes(b))
}

fn u64_at(file: &[u8], at: usize) -> Result<u64, String> {
    let b: [u8; 8] = bytes(file, at, 8)?
        .try_into()
        .map_err(|_| "a field of the wrong size".to_string())?;
    Ok(u64::from_le_bytes(b))
}

/// `at + by`, an offset in the file.
fn add(at: usize, by: usize) -> Result<usize, String> {
    at.checked_add(by)
        .ok_or_else(|| "malformed: an offset beyond any file".to_string())
}

/// A field of the file as an offset.
fn offset(value: u32) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| "malformed: an offset beyond any file".to_string())
}

/// A section's place in memory and in the file.
struct Section {
    /// Its relative virtual address.
    address: u32,
    /// How much of memory it covers.
    span: u32,
    /// How much of it the file holds, and where.
    raw_size: u32,
    raw_at: u32,
}

/// Where in the file the relative virtual address `rva` is.
fn file_offset(sections: &[Section], rva: u32) -> Result<usize, String> {
    for section in sections {
        let Some(within) = rva.checked_sub(section.address) else {
            continue;
        };
        if within >= section.span {
            continue;
        }
        if within >= section.raw_size {
            return Err(format!(
                "malformed: address {rva:#x} lies in the part of a section the file does not hold"
            ));
        }
        let at = u64::from(section.raw_at)
            .checked_add(u64::from(within))
            .and_then(|at| usize::try_from(at).ok());
        return at.ok_or_else(|| "malformed: an offset beyond any file".to_string());
    }
    Err(format!("malformed: address {rva:#x} lies in no section"))
}

/// The DLL name at `at`: printable ASCII up to a zero byte.
fn name_at(file: &[u8], at: usize) -> Result<String, String> {
    let rest = file
        .get(at..)
        .ok_or_else(|| format!("malformed: a DLL name at {at:#x}, beyond the end of the file"))?;
    let end = rest
        .iter()
        .take(NAME_MOST.saturating_add(1))
        .position(|b| *b == 0)
        .ok_or_else(|| {
            format!("malformed: the DLL name at {at:#x} does not end within {NAME_MOST} bytes")
        })?;
    let name = rest
        .get(..end)
        .ok_or_else(|| "malformed: a DLL name".to_string())?;
    if name.is_empty() || !name.iter().all(|b| (0x20..0x7f).contains(b)) {
        return Err(format!(
            "malformed: the DLL name at {at:#x} is empty or not printable ASCII"
        ));
    }
    String::from_utf8(name.to_vec()).map_err(|_| "malformed: a DLL name".to_string())
}

/// The DLL names of a table of `entry`-byte entries at `rva` (none when
/// `rva` is 0), ended by an entry of zeros; `name_rva` reads an entry's
/// name. An entry that names nothing but is not all zeros, a table without
/// its end, or one longer than [`ENTRIES_MOST`] is malformed.
fn table_names(
    file: &[u8],
    sections: &[Section],
    rva: u32,
    entry: usize,
    what: &str,
    name_rva: &dyn Fn(&[u8]) -> Result<u32, String>,
) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    if rva == 0 {
        return Ok(names);
    }
    let start = file_offset(sections, rva).map_err(|e| format!("the {what} table: {e}"))?;
    for index in 0..ENTRIES_MOST {
        let at = index
            .checked_mul(entry)
            .and_then(|by| start.checked_add(by))
            .ok_or_else(|| format!("the {what} table: an offset beyond any file"))?;
        let fields = bytes(file, at, entry).map_err(|e| format!("the {what} table: {e}"))?;
        if fields.iter().all(|b| *b == 0) {
            return Ok(names);
        }
        let name = name_rva(fields)?;
        if name == 0 {
            return Err(format!(
                "malformed: entry {index} of the {what} table names no DLL"
            ));
        }
        let at = file_offset(sections, name).map_err(|e| format!("the {what} table: {e}"))?;
        names.push(name_at(file, at)?);
    }
    Err(format!(
        "malformed: the {what} table has more than {ENTRIES_MOST} entries"
    ))
}

/// Reads the machine, the subsystem and the imported and delay-loaded
/// DLLs of a Windows program (a PE32 or PE32+ image).
pub fn read(file: &[u8]) -> Result<Image, String> {
    if file.get(..2) != Some(b"MZ".as_slice()) {
        return Err("not a Windows program: it does not start with `MZ`".to_string());
    }
    let pe = offset(u32_at(file, 0x3c)?)?;
    if bytes(file, pe, 4)? != b"PE\0\0" {
        return Err(
            "not a Windows program: no `PE` signature where its DOS header points".to_string(),
        );
    }
    let coff = add(pe, 4)?;
    let machine = u16_at(file, coff)?;
    let section_count = usize::from(u16_at(file, add(coff, 2)?)?);
    let optional_size = usize::from(u16_at(file, add(coff, 16)?)?);
    let optional = add(coff, 20)?;
    // Where the data directories start, and the image base, by the
    // optional header's kind.
    let (directories, image_base) = match u16_at(file, optional)? {
        PE32 => (96, u64::from(u32_at(file, add(optional, 28)?)?)),
        PE32_PLUS => (112, u64_at(file, add(optional, 24)?)?),
        other => {
            return Err(format!(
                "not a Windows program: an optional header of kind {other:#x}"
            ));
        }
    };
    if optional_size < directories {
        return Err(format!(
            "malformed: an optional header of {optional_size} bytes, too short for a program"
        ));
    }
    let subsystem = u16_at(file, add(optional, 68)?)?;
    let declared = offset(u32_at(file, add(optional, directories - 4)?)?)?;
    let present = declared.min((optional_size - directories) / 8);
    let directory = |index: usize| -> Result<u32, String> {
        if index >= present {
            return Ok(0);
        }
        u32_at(file, add(add(optional, directories)?, index * 8)?)
    };
    let table = add(optional, optional_size)?;
    let mut sections = Vec::new();
    for index in 0..section_count {
        let at = add(table, index * 40)?;
        let span = u32_at(file, add(at, 8)?)?;
        let raw_size = u32_at(file, add(at, 16)?)?;
        sections.push(Section {
            address: u32_at(file, add(at, 12)?)?,
            span: if span == 0 { raw_size } else { span },
            raw_size,
            raw_at: u32_at(file, add(at, 20)?)?,
        });
    }
    // An import descriptor: the name's address at 12 of its 20 bytes.
    let imports = table_names(
        file,
        &sections,
        directory(IMPORT_DIRECTORY)?,
        20,
        "import",
        &|entry| u32_at(entry, 12),
    )?;
    // A delay-load descriptor: its attributes at 0 and the name at 4 of
    // its 32 bytes. The name is an address relative to the image when
    // bit 0 of the attributes is set (every linker since Visual C++ 7),
    // and an absolute one in the oldest form, which only a 32-bit image's
    // field can hold.
    let delayed = table_names(
        file,
        &sections,
        directory(DELAY_IMPORT_DIRECTORY)?,
        32,
        "delay-load import",
        &|entry| {
            let attributes = u32_at(entry, 0)?;
            let name = u32_at(entry, 4)?;
            if attributes & 1 == 0 && image_base != 0 && u64::from(name) >= image_base {
                u32::try_from(u64::from(name) - image_base)
                    .map_err(|_| "malformed: a delay-load DLL name's address".to_string())
            } else {
                Ok(name)
            }
        },
    )?;
    Ok(Image {
        machine,
        subsystem,
        imports,
        delayed,
    })
}

/// Windows programs made in the tests, laid out as a linker lays them
/// out, with no more than [`read`] looks at.
#[cfg(test)]
pub(crate) mod synthetic {
    /// Where the one section is in memory and in the file.
    pub const SECTION_RVA: u32 = 0x1000;
    pub const SECTION_AT: usize = 0x200;
    /// Where the optional header starts.
    pub const OPTIONAL_AT: usize = 0x58;
    /// The image base of a 32-bit image.
    pub const BASE_32: u32 = 0x40_0000;

    /// A program for x86-64 (PE32+), or for x86 (PE32) when `pe32_plus`
    /// is false, of `subsystem`, importing `imports` and delay-loading
    /// `delayed`. Its one section holds the import descriptors, then the
    /// delay-load descriptors, then the names; the file ends with the last
    /// name's zero byte.
    pub fn image(pe32_plus: bool, subsystem: u16, imports: &[&str], delayed: &[&str]) -> Vec<u8> {
        let directories = if pe32_plus { 112 } else { 96 };
        let optional_size = directories + 16 * 8;
        let imports_len = if imports.is_empty() {
            0
        } else {
            (imports.len() + 1) * 20
        };
        let delayed_len = if delayed.is_empty() {
            0
        } else {
            (delayed.len() + 1) * 32
        };
        let names_at = imports_len + delayed_len;
        let mut names = Vec::new();
        let mut name_rvas = Vec::new();
        for dll in imports.iter().chain(delayed) {
            name_rvas.push(SECTION_RVA + (names_at + names.len()) as u32);
            names.extend_from_slice(dll.as_bytes());
            names.push(0);
        }
        let mut section = vec![0u8; names_at];
        let put = |section: &mut Vec<u8>, at: usize, value: u32| {
            section[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        for (index, rva) in name_rvas[..imports.len()].iter().enumerate() {
            let at = index * 20;
            // The lookup and address tables are not read: any address.
            put(&mut section, at, SECTION_RVA + 0x800);
            put(&mut section, at + 12, *rva);
            put(&mut section, at + 16, SECTION_RVA + 0x900);
        }
        for (index, rva) in name_rvas[imports.len()..].iter().enumerate() {
            let at = imports_len + index * 32;
            put(&mut section, at, 1);
            put(&mut section, at + 4, *rva);
            put(&mut section, at + 12, SECTION_RVA + 0xa00);
        }
        section.extend(names);

        let mut file = vec![0u8; SECTION_AT];
        file[..2].copy_from_slice(b"MZ");
        file[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        file[0x40..0x44].copy_from_slice(b"PE\0\0");
        let machine: u16 = if pe32_plus { 0x8664 } else { 0x014c };
        file[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
        file[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
        file[0x54..0x56].copy_from_slice(&(optional_size as u16).to_le_bytes());
        let o = OPTIONAL_AT;
        if pe32_plus {
            file[o..o + 2].copy_from_slice(&0x20bu16.to_le_bytes());
            file[o + 24..o + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        } else {
            file[o..o + 2].copy_from_slice(&0x10bu16.to_le_bytes());
            file[o + 28..o + 32].copy_from_slice(&BASE_32.to_le_bytes());
        }
        file[o + 68..o + 70].copy_from_slice(&subsystem.to_le_bytes());
        file[o + directories - 4..o + directories].copy_from_slice(&16u32.to_le_bytes());
        let directory = |file: &mut Vec<u8>, index: usize, rva: u32, len: usize| {
            let at = o + directories + index * 8;
            file[at..at + 4].copy_from_slice(&rva.to_le_bytes());
            file[at + 4..at + 8].copy_from_slice(&(len as u32).to_le_bytes());
        };
        if !imports.is_empty() {
            directory(&mut file, 1, SECTION_RVA, imports_len);
        }
        if !delayed.is_empty() {
            directory(&mut file, 13, SECTION_RVA + imports_len as u32, delayed_len);
        }
        let s = o + optional_size;
        assert!(s + 40 <= SECTION_AT);
        file[s..s + 8].copy_from_slice(b".idata\0\0");
        let len = section.len() as u32;
        file[s + 8..s + 12].copy_from_slice(&len.to_le_bytes());
        file[s + 12..s + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
        file[s + 16..s + 20].copy_from_slice(&len.to_le_bytes());
        file[s + 20..s + 24].copy_from_slice(&(SECTION_AT as u32).to_le_bytes());
        file.extend(section);
        file
    }
}

#[cfg(test)]
mod tests {
    use super::synthetic::{BASE_32, OPTIONAL_AT, SECTION_AT, image};
    use super::*;

    #[test]
    fn the_imports_the_delay_loads_and_the_subsystem_are_read() {
        let file = image(
            true,
            CONSOLE,
            &["KERNEL32.dll", "VCRUNTIME140.dll", "ntdll.dll"],
            &["USER32.dll"],
        );
        let found = read(&file).unwrap();
        assert_eq!(
            found,
            Image {
                machine: 0x8664,
                subsystem: CONSOLE,
                imports: vec![
                    "KERNEL32.dll".to_string(),
                    "VCRUNTIME140.dll".to_string(),
                    "ntdll.dll".to_string()
                ],
                delayed: vec!["USER32.dll".to_string()],
            }
        );
        assert_eq!(machine_name(found.machine), "x86-64");
        assert_eq!(subsystem_name(found.subsystem), "console subsystem");
        // A program with windows only, and a 32-bit one.
        let gui = read(&image(true, GUI, &["KERNEL32.dll"], &[])).unwrap();
        assert_eq!(gui.subsystem, GUI);
        assert_eq!(subsystem_name(gui.subsystem), "Windows GUI subsystem");
        assert!(gui.delayed.is_empty());
        let x86 = read(&image(false, CONSOLE, &["KERNEL32.dll"], &["a.dll"])).unwrap();
        assert_eq!(x86.machine, 0x014c);
        assert_eq!(machine_name(x86.machine), "x86");
        assert_eq!(x86.imports, ["KERNEL32.dll"]);
        assert_eq!(x86.delayed, ["a.dll"]);
        // No import table at all.
        let none = read(&image(true, CONSOLE, &[], &[])).unwrap();
        assert!(none.imports.is_empty() && none.delayed.is_empty());
    }

    /// The oldest delay-load form gives the name as an absolute address
    /// in a 32-bit image (bit 0 of the attributes clear).
    #[test]
    fn an_absolute_delay_load_name_is_read_relative_to_the_image_base() {
        let mut file = image(false, CONSOLE, &["KERNEL32.dll"], &["VCOMP140.DLL"]);
        let entry = SECTION_AT + 2 * 20;
        let rva = u32::from_le_bytes(file[entry + 4..entry + 8].try_into().unwrap());
        file[entry..entry + 4].copy_from_slice(&0u32.to_le_bytes());
        file[entry + 4..entry + 8].copy_from_slice(&(BASE_32 + rva).to_le_bytes());
        assert_eq!(read(&file).unwrap().delayed, ["VCOMP140.DLL"]);
    }

    /// Every cut of a program, the last name's zero byte included, is an
    /// error: nothing is read beyond the end of the file.
    #[test]
    fn a_program_cut_short_is_an_error() {
        let file = image(
            true,
            CONSOLE,
            &["KERNEL32.dll", "VCRUNTIME140.dll"],
            &["USER32.dll"],
        );
        assert!(read(&file).is_ok());
        for cut in 0..file.len() {
            assert!(read(&file[..cut]).is_err(), "cut at {cut}");
        }
    }

    /// Junk, and programs whose fields point anywhere, are errors and
    /// never a panic.
    #[test]
    fn nonsense_is_an_error_and_never_a_panic() {
        let good = image(true, CONSOLE, &["KERNEL32.dll", "VCRUNTIME140.dll"], &[]);
        let mut cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", Vec::new()),
            ("MZ alone", b"MZ".to_vec()),
            ("an ELF file", b"\x7fELF\x02\x01\x01\0".to_vec()),
            ("all ones", vec![0xff; 4096]),
            ("all zeros", vec![0; 4096]),
        ];
        let patched = |at: usize, value: &[u8]| {
            let mut file = good.clone();
            file[at..at + value.len()].copy_from_slice(value);
            file
        };
        let o = OPTIONAL_AT;
        let first_name = SECTION_AT + 3 * 20;
        cases.extend([
            ("e_lfanew far away", patched(0x3c, &u32::MAX.to_le_bytes())),
            (
                "e_lfanew at the end",
                patched(0x3c, &(good.len() as u32 - 2).to_le_bytes()),
            ),
            ("no PE signature", patched(0x40, b"PX\0\0")),
            (
                "an unknown optional header",
                patched(o, &0x30bu16.to_le_bytes()),
            ),
            (
                "an optional header too short",
                patched(0x54, &8u16.to_le_bytes()),
            ),
            (
                "an optional header too long",
                patched(0x54, &u16::MAX.to_le_bytes()),
            ),
            ("every section", patched(0x46, &u16::MAX.to_le_bytes())),
            ("no section", patched(0x46, &0u16.to_le_bytes())),
            (
                "imports in no section",
                patched(o + 112 + 8, &0x9000u32.to_le_bytes()),
            ),
            (
                "imports at the top of memory",
                patched(o + 112 + 8, &u32::MAX.to_le_bytes()),
            ),
            (
                "a section far away",
                patched(o + 240 + 20, &u32::MAX.to_le_bytes()),
            ),
            (
                "a section too small for its names",
                patched(o + 240 + 8, &8u32.to_le_bytes()),
            ),
            (
                "a section the file does not hold",
                patched(o + 240 + 16, &8u32.to_le_bytes()),
            ),
            (
                "a name in no section",
                patched(SECTION_AT + 12, &0x7000u32.to_le_bytes()),
            ),
            (
                "a name of nothing",
                patched(SECTION_AT + 12, &0u32.to_le_bytes()),
            ),
            (
                "a name that is not ASCII",
                patched(first_name, &[0xc3, 0xa9]),
            ),
            ("an empty name", patched(first_name, &[0])),
        ]);
        // A name without its end: the last zero byte replaced.
        let mut endless = good.clone();
        let last = endless.len() - 1;
        endless[last] = b'x';
        cases.push(("a name without its end", endless));
        // A table without its end: the terminating entry replaced by a
        // copy of the first, so the table runs on into the names.
        let mut unended = good.clone();
        let first = unended[SECTION_AT..SECTION_AT + 20].to_vec();
        unended[SECTION_AT + 40..SECTION_AT + 60].copy_from_slice(&first);
        cases.push(("a table without its end", unended));
        for (what, file) in &cases {
            assert!(read(file).is_err(), "{what}: {:?}", read(file));
        }
        // More entries than any program has.
        let many: Vec<String> = (0..=ENTRIES_MOST).map(|i| format!("d{i}.dll")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let error = read(&image(true, CONSOLE, &many, &[])).unwrap_err();
        assert!(error.contains("more than 4096"), "{error}");
        // Seeded junk, and junk behind a real header: whatever it reads,
        // it returns.
        let mut seed: u64 = 0x5eed_3dc4_af70_0001;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u8
        };
        for round in 0..2000 {
            let len = usize::from(next()) * 4 + usize::from(next());
            let mut file: Vec<u8> = (0..len).map(|_| next()).collect();
            if round % 2 == 1 {
                let keep = (SECTION_AT + usize::from(next())).min(good.len());
                let mut headed = good[..keep].to_vec();
                headed.extend(file);
                let flip = usize::from(next()) * 2 % headed.len();
                headed[flip] ^= next() | 1;
                file = headed;
                let _ = read(&file);
            } else {
                assert!(read(&file).is_err() || file.starts_with(b"MZ"));
            }
        }
        assert_eq!(read(&good).unwrap().imports.len(), 2);
    }
}
