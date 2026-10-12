//! A minimal ZIP container for stored (uncompressed) entries.
//!
//! Standalone: depends on no workspace crate. It is the container of 3MF
//! packages (`craft3d-meshio`) and of the native `.3dcraft` file
//! (`craft3d-format`), and both need nothing more than "a few named byte
//! strings in one file".
//!
//! Layout per the PKWARE APPNOTE: local file headers with their data, then
//! the central directory, then the end-of-central-directory record.
//!
//! - [`write_stored`] is deterministic: the same entries give the same bytes
//!   (fixed timestamp, no host-dependent fields).
//! - [`read_stored`] is written for hostile input: every offset, size and
//!   count in the file is checked before it is used, and anything this crate
//!   does not implement (compression, encryption, ZIP64, multi-disk
//!   archives) is refused with an error that says so. It never panics and
//!   never allocates more than a small multiple of the entry count.
//!
//! Not supported, by design: compression, ZIP64 (sizes and offsets must fit
//! 32 bits), encryption, archives split over several disks, and data before
//! the first entry (self-extracting archives).

use std::collections::BTreeSet;
use std::fmt;

/// Largest number of entries [`read_stored`] accepts and [`write_stored`]
/// writes. The users of this crate store a handful of entries; the cap keeps
/// a hostile central directory from costing more than it is worth.
pub const MAX_ENTRIES: usize = 4096;

/// What [`write_stored`] and [`read_stored`] can report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZipError {
    /// Writing: the archive, an entry or a name does not fit the 32-bit
    /// (16-bit for names) fields of a ZIP without ZIP64.
    TooLarge,
    /// More entries than [`MAX_ENTRIES`].
    TooManyEntries {
        count: usize,
    },
    /// No end-of-central-directory record: not a ZIP archive, or cut short.
    NoEndRecord,
    /// The archive uses ZIP64 fields.
    Zip64,
    /// The archive is one part of a multi-disk archive.
    MultiDisk,
    /// A record does not start with its signature, or the records do not
    /// add up (the string says which).
    Corrupt(&'static str),
    /// An offset or size points outside the file (the string says which).
    OutOfBounds(&'static str),
    /// The entry is compressed; only method 0 (stored) is read.
    Compressed {
        name: String,
        method: u16,
    },
    Encrypted {
        name: String,
    },
    /// The local header of the entry contradicts the central directory.
    HeaderMismatch {
        name: String,
    },
    /// The data of the entry does not have the CRC-32 the archive records.
    CrcMismatch {
        name: String,
        expected: u32,
        actual: u32,
    },
    NameNotUtf8,
    /// A name a careful reader does not accept: empty, absolute, with a
    /// `..` component, a backslash or a NUL.
    InvalidName {
        name: String,
        reason: &'static str,
    },
    DuplicateName(String),
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipError::TooLarge => write!(
                f,
                "too large for a ZIP archive: 4 GiB or more of data, or a name of 64 KiB or more \
                 (ZIP64 is not supported)"
            ),
            ZipError::TooManyEntries { count } => write!(
                f,
                "the ZIP archive has {count} entries; at most {MAX_ENTRIES} are supported"
            ),
            ZipError::NoEndRecord => write!(
                f,
                "no end-of-central-directory record: this is not a ZIP archive, or it was cut short"
            ),
            ZipError::Zip64 => write!(f, "ZIP64 archives are not supported"),
            ZipError::MultiDisk => write!(f, "multi-disk ZIP archives are not supported"),
            ZipError::Corrupt(what) => write!(f, "the ZIP archive is damaged: {what}"),
            ZipError::OutOfBounds(what) => {
                write!(
                    f,
                    "the ZIP archive is damaged: {what} lies outside the file"
                )
            }
            ZipError::Compressed { name, method } => write!(
                f,
                "ZIP entry `{name}` is compressed (method {method}); compression is not \
                 supported, only stored entries can be read"
            ),
            ZipError::Encrypted { name } => write!(
                f,
                "ZIP entry `{name}` is encrypted; encryption is not supported"
            ),
            ZipError::HeaderMismatch { name } => write!(
                f,
                "the ZIP archive is damaged: the local header of `{name}` contradicts the \
                 central directory"
            ),
            ZipError::CrcMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "ZIP entry `{name}` is damaged: its CRC-32 is {actual:08x}, the archive \
                 records {expected:08x}"
            ),
            ZipError::NameNotUtf8 => write!(f, "a ZIP entry has a name that is not UTF-8"),
            ZipError::InvalidName { name, reason } => {
                write!(f, "ZIP entry name `{name}` is not accepted: {reason}")
            }
            ZipError::DuplicateName(name) => {
                write!(f, "the ZIP archive has two entries named `{name}`")
            }
        }
    }
}

impl std::error::Error for ZipError {}

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320), bit by bit.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Version 2.0, the baseline every reader supports.
const VERSION: u16 = 20;
/// General-purpose bit 11: names are UTF-8.
const FLAGS: u16 = 1 << 11;
/// A fixed timestamp (1980-01-01 00:00), so output is reproducible.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

const LOCAL_SIGNATURE: &[u8; 4] = b"PK\x03\x04";
const CENTRAL_SIGNATURE: &[u8; 4] = b"PK\x01\x02";
const END_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";
/// Sizes of the fixed parts of the three records.
const LOCAL_LEN: usize = 30;
const CENTRAL_LEN: usize = 46;
const END_LEN: usize = 22;
const ZIP64_LOCATOR_LEN: usize = 20;

/// General-purpose bit 0: the entry is encrypted. Bit 6: strong encryption.
const FLAG_ENCRYPTED: u16 = 1 | (1 << 6);
/// General-purpose bit 3: the local header has zeros for CRC and sizes, the
/// real values follow the data. The central directory still has them.
const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;

/// The rule both the writer and the reader apply to names: relative, with
/// forward slashes, and nothing that could leave the directory an archive is
/// unpacked into.
fn check_name(name: &str) -> Result<(), ZipError> {
    let reason = if name.is_empty() {
        "it is empty"
    } else if name.contains('\0') {
        "it contains a NUL character"
    } else if name.contains('\\') {
        "it contains a backslash"
    } else if name.starts_with('/') {
        "it is an absolute path"
    } else if name.as_bytes().get(1) == Some(&b':')
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
    {
        "it starts with a drive letter"
    } else if name.split('/').any(|component| component == "..") {
        "it contains a `..` component"
    } else {
        return Ok(());
    };
    Err(ZipError::InvalidName {
        name: name.to_string(),
        reason,
    })
}

/// Writes the entries, in order, as a ZIP archive of stored entries.
///
/// Fails for names [`read_stored`] would refuse (so everything written here
/// reads back), for more than [`MAX_ENTRIES`] entries, and for data that
/// does not fit a ZIP without ZIP64.
pub fn write_stored(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, ZipError> {
    if entries.len() > MAX_ENTRIES {
        return Err(ZipError::TooManyEntries {
            count: entries.len(),
        });
    }
    let mut names = BTreeSet::new();
    for (name, _) in entries {
        check_name(name)?;
        if !names.insert(*name) {
            return Err(ZipError::DuplicateName((*name).to_string()));
        }
    }
    let count = u16::try_from(entries.len()).map_err(|_| ZipError::TooLarge)?;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = u32::try_from(out.len()).map_err(|_| ZipError::TooLarge)?;
        let size = u32::try_from(data.len()).map_err(|_| ZipError::TooLarge)?;
        let name_len = u16::try_from(name.len()).map_err(|_| ZipError::TooLarge)?;
        let crc = crc32(data);

        // Fields shared by the local header and the central directory entry.
        let mut common = Vec::with_capacity(26);
        common.extend_from_slice(&VERSION.to_le_bytes());
        common.extend_from_slice(&FLAGS.to_le_bytes());
        common.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
        common.extend_from_slice(&DOS_TIME.to_le_bytes());
        common.extend_from_slice(&DOS_DATE.to_le_bytes());
        common.extend_from_slice(&crc.to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes()); // compressed size
        common.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        common.extend_from_slice(&name_len.to_le_bytes());
        common.extend_from_slice(&0_u16.to_le_bytes()); // extra field length

        out.extend_from_slice(LOCAL_SIGNATURE);
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(CENTRAL_SIGNATURE);
        central.extend_from_slice(&VERSION.to_le_bytes()); // version made by
        central.extend_from_slice(&common);
        central.extend_from_slice(&0_u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0_u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0_u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0_u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = u32::try_from(out.len()).map_err(|_| ZipError::TooLarge)?;
    let central_size = u32::try_from(central.len()).map_err(|_| ZipError::TooLarge)?;
    // 0xFFFFFFFF in either field is the ZIP64 marker, so it is refused too.
    if central_offset == u32::MAX || central_size == u32::MAX {
        return Err(ZipError::TooLarge);
    }
    central_offset
        .checked_add(central_size)
        .ok_or(ZipError::TooLarge)?;
    out.extend_from_slice(&central);
    out.extend_from_slice(END_SIGNATURE);
    out.extend_from_slice(&0_u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0_u16.to_le_bytes()); // disk with the central directory
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes()); // comment length
    Ok(out)
}

/// One entry of an archive, borrowed from the bytes it was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

/// `len` bytes at `at`, or `None` when that range is not inside `bytes`
/// (including when `at + len` overflows).
fn slice(bytes: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    bytes.get(at..at.checked_add(len)?)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let raw: [u8; 2] = slice(bytes, at, 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(raw))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = slice(bytes, at, 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// A 32-bit file field as an index. `None` only where `usize` is narrower
/// than 32 bits.
fn index(field: u32) -> Option<usize> {
    usize::try_from(field).ok()
}

/// Position of the end-of-central-directory record: the last place where
/// its signature is followed by a comment that ends exactly at the end of
/// the file. The comment is at most 64 KiB, which bounds the search.
fn find_end_record(bytes: &[u8]) -> Option<usize> {
    let last = bytes.len().checked_sub(END_LEN)?;
    let first = last.saturating_sub(usize::from(u16::MAX));
    (first..=last).rev().find(|&at| {
        slice(bytes, at, 4) == Some(END_SIGNATURE)
            && u16_at(bytes, at.saturating_add(20)).map(usize::from) == last.checked_sub(at)
    })
}

/// Reads an archive of stored entries: finds the end-of-central-directory
/// record (a trailing archive comment is allowed), walks the central
/// directory and follows each entry to its local header. Entries come back
/// in central-directory order, borrowing their names and data from `bytes`.
///
/// Refused, each with its own [`ZipError`]: files that are cut short or
/// whose offsets and sizes point outside the file, compressed and encrypted
/// entries, ZIP64 and multi-disk archives, CRC mismatches, names that are
/// not UTF-8, duplicate names, names that are absolute or contain a `..`
/// component or a backslash, and more than [`MAX_ENTRIES`] entries.
///
/// Work is bounded: nothing is copied, the entry count is capped, and every
/// entry's data must lie before the central directory with the total not
/// exceeding the file size, so entries that overlap each other cannot make
/// a small file cost more CRC work than its own length.
pub fn read_stored(bytes: &[u8]) -> Result<Vec<Entry<'_>>, ZipError> {
    let end = find_end_record(bytes).ok_or(ZipError::NoEndRecord)?;
    let field16 = |at: usize| {
        end.checked_add(at)
            .and_then(|at| u16_at(bytes, at))
            .ok_or(ZipError::Corrupt("the end record is incomplete"))
    };
    let field32 = |at: usize| {
        end.checked_add(at)
            .and_then(|at| u32_at(bytes, at))
            .ok_or(ZipError::Corrupt("the end record is incomplete"))
    };
    let (this_disk, directory_disk) = (field16(4)?, field16(6)?);
    let (count_here, count) = (field16(8)?, field16(10)?);
    let (central_size, central_offset) = (field32(12)?, field32(16)?);

    // A ZIP64 archive puts a locator right before this record and all-ones
    // markers in the fields that moved to the ZIP64 records.
    let has_locator = end
        .checked_sub(ZIP64_LOCATOR_LEN)
        .and_then(|at| slice(bytes, at, 4))
        == Some(ZIP64_LOCATOR_SIGNATURE);
    if has_locator
        || count == u16::MAX
        || count_here == u16::MAX
        || central_size == u32::MAX
        || central_offset == u32::MAX
    {
        return Err(ZipError::Zip64);
    }
    if this_disk != 0 || directory_disk != 0 {
        return Err(ZipError::MultiDisk);
    }
    if count_here != count {
        return Err(ZipError::Corrupt("the entry counts disagree"));
    }
    let count = usize::from(count);
    if count > MAX_ENTRIES {
        return Err(ZipError::TooManyEntries { count });
    }

    const DIRECTORY: ZipError = ZipError::OutOfBounds("the central directory");
    let central_offset = index(central_offset).ok_or(DIRECTORY)?;
    let central_size = index(central_size).ok_or(DIRECTORY)?;
    let central = slice(bytes, central_offset, central_size).ok_or(DIRECTORY)?;
    if central_offset.checked_add(central_size) != Some(end) {
        return Err(ZipError::Corrupt(
            "the central directory does not end at the end record",
        ));
    }

    let mut entries = Vec::with_capacity(count);
    let mut names = BTreeSet::new();
    let mut total: usize = 0;
    let mut at: usize = 0;
    for _ in 0..count {
        const ENTRY: ZipError = ZipError::Corrupt("the central directory is cut short");
        let header = slice(central, at, CENTRAL_LEN).ok_or(ENTRY)?;
        if slice(header, 0, 4) != Some(CENTRAL_SIGNATURE) {
            return Err(ZipError::Corrupt(
                "a central directory entry has the wrong signature",
            ));
        }
        let flags = u16_at(header, 8).ok_or(ENTRY)?;
        let method = u16_at(header, 10).ok_or(ENTRY)?;
        let crc = u32_at(header, 16).ok_or(ENTRY)?;
        let compressed_size = u32_at(header, 20).ok_or(ENTRY)?;
        let size = u32_at(header, 24).ok_or(ENTRY)?;
        let name_len = usize::from(u16_at(header, 28).ok_or(ENTRY)?);
        let extra_len = usize::from(u16_at(header, 30).ok_or(ENTRY)?);
        let comment_len = usize::from(u16_at(header, 32).ok_or(ENTRY)?);
        let disk = u16_at(header, 34).ok_or(ENTRY)?;
        let local_offset = u32_at(header, 42).ok_or(ENTRY)?;

        let name_at = at.checked_add(CENTRAL_LEN).ok_or(ENTRY)?;
        let name_bytes = slice(central, name_at, name_len).ok_or(ENTRY)?;
        at = name_at
            .checked_add(name_len)
            .and_then(|at| at.checked_add(extra_len))
            .and_then(|at| at.checked_add(comment_len))
            .filter(|&at| at <= central.len())
            .ok_or(ENTRY)?;

        let name = std::str::from_utf8(name_bytes).map_err(|_| ZipError::NameNotUtf8)?;
        check_name(name)?;
        let owned = || name.to_string();
        if flags & FLAG_ENCRYPTED != 0 {
            return Err(ZipError::Encrypted { name: owned() });
        }
        if method != 0 {
            return Err(ZipError::Compressed {
                name: owned(),
                method,
            });
        }
        if compressed_size == u32::MAX || size == u32::MAX || local_offset == u32::MAX {
            return Err(ZipError::Zip64);
        }
        if disk != 0 {
            return Err(ZipError::MultiDisk);
        }
        if compressed_size != size {
            return Err(ZipError::Corrupt(
                "a stored entry has different compressed and uncompressed sizes",
            ));
        }
        if !names.insert(name) {
            return Err(ZipError::DuplicateName(owned()));
        }

        // The local header: it repeats the name, and the data follows it.
        const LOCAL: ZipError = ZipError::OutOfBounds("a local file header");
        let local_at = index(local_offset).ok_or(LOCAL)?;
        let local = slice(bytes, local_at, LOCAL_LEN).ok_or(LOCAL)?;
        if slice(local, 0, 4) != Some(LOCAL_SIGNATURE) {
            return Err(ZipError::Corrupt(
                "a local file header has the wrong signature",
            ));
        }
        let local_flags = u16_at(local, 6).ok_or(LOCAL)?;
        let local_method = u16_at(local, 8).ok_or(LOCAL)?;
        let local_name_len = usize::from(u16_at(local, 26).ok_or(LOCAL)?);
        let local_extra_len = usize::from(u16_at(local, 28).ok_or(LOCAL)?);
        if local_flags & FLAG_ENCRYPTED != 0 {
            return Err(ZipError::Encrypted { name: owned() });
        }
        if local_method != 0 {
            return Err(ZipError::Compressed {
                name: owned(),
                method: local_method,
            });
        }
        let local_name_at = local_at.checked_add(LOCAL_LEN).ok_or(LOCAL)?;
        let local_name = slice(bytes, local_name_at, local_name_len).ok_or(LOCAL)?;
        // With a data descriptor the local CRC and sizes are zero and the
        // central directory is the only place that has them.
        let sizes_agree = local_flags & FLAG_DATA_DESCRIPTOR != 0
            || (u32_at(local, 14) == Some(crc)
                && u32_at(local, 18) == Some(size)
                && u32_at(local, 22) == Some(size));
        if local_name != name_bytes || !sizes_agree {
            return Err(ZipError::HeaderMismatch { name: owned() });
        }

        const DATA: ZipError = ZipError::OutOfBounds("the data of an entry");
        let size = index(size).ok_or(DATA)?;
        let data_at = local_name_at
            .checked_add(local_name_len)
            .and_then(|at| at.checked_add(local_extra_len))
            .ok_or(DATA)?;
        let data = slice(bytes, data_at, size).ok_or(DATA)?;
        // Data belongs before the central directory; and entries that share
        // bytes would let a small file demand unbounded CRC work.
        if data_at.checked_add(size).is_none_or(|e| e > central_offset) {
            return Err(DATA);
        }
        total = total.saturating_add(size);
        if total > bytes.len() {
            return Err(ZipError::Corrupt("entries overlap"));
        }
        let actual = crc32(data);
        if actual != crc {
            return Err(ZipError::CrcMismatch {
                name: owned(),
                expected: crc,
                actual,
            });
        }
        entries.push(Entry { name, data });
    }
    if at != central.len() {
        return Err(ZipError::Corrupt(
            "the central directory is longer than its entries",
        ));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests;
