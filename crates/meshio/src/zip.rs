//! A minimal ZIP writer: stored (uncompressed) entries only, which is all a
//! 3MF package needs. Layout per the PKWARE APPNOTE: local file headers with
//! data, then the central directory, then the end-of-central-directory
//! record. No ZIP64: sizes and offsets must fit 32 bits.

use crate::MeshError;

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320), bit by bit.
pub(crate) fn crc32(data: &[u8]) -> u32 {
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

const TOO_LARGE: MeshError =
    MeshError::TooLarge("a 3MF package of 4 GiB or more (ZIP64 is not supported)");

/// Version 2.0, the baseline every reader supports.
const VERSION: u16 = 20;
/// General-purpose bit 11: names are UTF-8.
const FLAGS: u16 = 1 << 11;
/// A fixed timestamp (1980-01-01 00:00), so output is reproducible.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

pub(crate) fn write_stored(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, MeshError> {
    let count = u16::try_from(entries.len()).map_err(|_| TOO_LARGE)?;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = u32::try_from(out.len()).map_err(|_| TOO_LARGE)?;
        let size = u32::try_from(data.len()).map_err(|_| TOO_LARGE)?;
        let name_len = u16::try_from(name.len()).map_err(|_| TOO_LARGE)?;
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

        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&VERSION.to_le_bytes()); // version made by
        central.extend_from_slice(&common);
        central.extend_from_slice(&0_u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0_u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0_u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0_u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = u32::try_from(out.len()).map_err(|_| TOO_LARGE)?;
    let central_size = u32::try_from(central.len()).map_err(|_| TOO_LARGE)?;
    central_offset.checked_add(central_size).ok_or(TOO_LARGE)?;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&0_u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0_u16.to_le_bytes()); // disk with the central directory
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes()); // comment length
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn u16_at(b: &[u8], i: usize) -> usize {
        u16::from_le_bytes(b[i..i + 2].try_into().unwrap()) as usize
    }

    fn u32_at(b: &[u8], i: usize) -> usize {
        u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize
    }

    /// A minimal reader that walks the central directory, follows each
    /// entry to its local header and verifies sizes and CRCs.
    pub(crate) fn read_stored(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let end = bytes.len() - 22;
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        let count = u16_at(bytes, end + 10);
        let (size, mut at) = (u32_at(bytes, end + 12), u32_at(bytes, end + 16));
        assert_eq!(
            at + size,
            end,
            "central directory must end at the end record"
        );
        let mut entries = Vec::new();
        for _ in 0..count {
            assert_eq!(&bytes[at..at + 4], b"PK\x01\x02");
            assert_eq!(u16_at(bytes, at + 10), 0, "stored");
            let (crc, len) = (u32_at(bytes, at + 16), u32_at(bytes, at + 24));
            assert_eq!(u32_at(bytes, at + 20), len);
            let name_len = u16_at(bytes, at + 28);
            let local = u32_at(bytes, at + 42);
            let name = std::str::from_utf8(&bytes[at + 46..at + 46 + name_len]).unwrap();

            assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
            assert_eq!(
                &bytes[local + 4..local + 30],
                &bytes[at + 6..at + 32],
                "headers disagree"
            );
            let data_at = local + 30 + name_len;
            assert_eq!(&bytes[local + 30..data_at], name.as_bytes());
            let data = &bytes[data_at..data_at + len];
            assert_eq!(crc32(data) as usize, crc);
            entries.push((name.to_string(), data.to_vec()));
            at += 46 + name_len;
        }
        assert_eq!(at, end);
        entries
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn entries_read_back_and_output_is_reproducible() {
        let entries: [(&str, &[u8]); 3] = [
            ("a.txt", b"hello"),
            ("dir/ü.bin", &[0, 255, 7]),
            ("empty", b""),
        ];
        let bytes = write_stored(&entries).unwrap();
        let back = read_stored(&bytes);
        assert_eq!(back.len(), 3);
        for ((name, data), (got_name, got_data)) in entries.iter().zip(&back) {
            assert_eq!(name, got_name);
            assert_eq!(data, &got_data.as_slice());
        }
        assert_eq!(bytes, write_stored(&entries).unwrap());
        assert_eq!(read_stored(&write_stored(&[]).unwrap()).len(), 0);
    }
}
