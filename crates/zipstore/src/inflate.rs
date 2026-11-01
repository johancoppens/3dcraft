//! DEFLATE decompression (RFC 1951), for reading the compressed entries of
//! archives other programs write (FreeCAD's `.FCStd`). Written from the
//! RFC: stored, fixed-Huffman and dynamic-Huffman blocks, the canonical
//! codes assigned as its §3.2.2 assigns them and decoded one bit at a
//! time.
//!
//! Hostile input: every code, length and distance is checked; the output
//! never grows past the limit the caller gives (the size the archive
//! declares), so a small entry cannot expand without bound.

/// Why a stream could not be decompressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InflateError {
    /// The stream ends before its last block does.
    Truncated,
    /// The output would grow past the limit.
    TooLong,
    /// The stream is not valid DEFLATE (the string says where).
    Invalid(&'static str),
}

impl std::fmt::Display for InflateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InflateError::Truncated => write!(f, "the compressed data is cut short"),
            InflateError::TooLong => {
                write!(
                    f,
                    "the compressed data expands past the size the archive records"
                )
            }
            InflateError::Invalid(what) => write!(f, "the compressed data is damaged: {what}"),
        }
    }
}

const MAX_BITS: usize = 15;

struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    /// Bits not yet used, low bits first.
    buffer: u32,
    count: u32,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, InflateError> {
        if self.count == 0 {
            let byte = *self.data.get(self.at).ok_or(InflateError::Truncated)?;
            self.at += 1;
            self.buffer = u32::from(byte);
            self.count = 8;
        }
        let b = self.buffer & 1;
        self.buffer >>= 1;
        self.count -= 1;
        Ok(b)
    }

    /// `n` bits (at most 16), the first read the lowest.
    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        let mut value = 0;
        for k in 0..n {
            value |= self.bit()? << k;
        }
        Ok(value)
    }

    /// Drops what is left of the current byte.
    fn align(&mut self) {
        self.buffer = 0;
        self.count = 0;
    }
}

/// A canonical Huffman code as RFC 1951 §3.2.2 assigns it: the codes of
/// each bit length are consecutive, starting at `next_code[len]`
/// (computed from the count of codes of every shorter length), given to
/// the symbols of that length in symbol order. Decoding reads one bit at
/// a time and, at each length, asks whether the code read so far falls
/// in that length's run of codes.
struct Huffman {
    /// Per length: the first code of that length and how many there are.
    first: [u32; MAX_BITS + 1],
    count: [u32; MAX_BITS + 1],
    /// Per length: where its symbols begin in `symbols`.
    start: [usize; MAX_BITS + 1],
    /// The symbols, by length and then symbol order.
    symbols: Vec<u16>,
}

impl Huffman {
    /// The code of `lengths` (0 for a symbol without a code). A set of
    /// lengths that would need more codes than a length allows is
    /// refused; an incomplete one is allowed (RFC 1951 permits one
    /// distance code), its unused codes refused when met.
    fn new(lengths: &[u8]) -> Result<Huffman, InflateError> {
        let mut count = [0u32; MAX_BITS + 1];
        for &l in lengths {
            if l != 0 {
                let slot = count
                    .get_mut(usize::from(l))
                    .ok_or(InflateError::Invalid("a code length above 15"))?;
                *slot += 1;
            }
        }
        // RFC 1951 §3.2.2, step 2: the first code of each length.
        let mut first = [0u32; MAX_BITS + 1];
        let mut code = 0u32;
        for len in 1..=MAX_BITS {
            // `count[0]` is 0: lengths of 0 are no codes.
            code = (code + count.get(len - 1).copied().unwrap_or(0)) << 1;
            let here = count.get(len).copied().unwrap_or(0);
            if code + here > (1u32 << len) {
                return Err(InflateError::Invalid("an over-subscribed set of codes"));
            }
            if let Some(slot) = first.get_mut(len) {
                *slot = code;
            }
        }
        let mut start = [0usize; MAX_BITS + 1];
        let mut at = 0usize;
        for len in 1..=MAX_BITS {
            if let Some(slot) = start.get_mut(len) {
                *slot = at;
            }
            at += count.get(len).copied().unwrap_or(0) as usize;
        }
        let mut symbols = vec![0u16; at];
        let mut filled = start;
        for (symbol, &l) in lengths.iter().enumerate() {
            if l == 0 {
                continue;
            }
            let Some(next) = filled.get_mut(usize::from(l)) else {
                continue;
            };
            if let Some(slot) = symbols.get_mut(*next) {
                *slot = u16::try_from(symbol).unwrap_or(u16::MAX);
            }
            *next += 1;
        }
        Ok(Huffman {
            first,
            count,
            start,
            symbols,
        })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, InflateError> {
        let mut code = 0u32;
        for len in 1..=MAX_BITS {
            code = (code << 1) | bits.bit()?;
            let (Some(&first), Some(&count), Some(&start)) = (
                self.first.get(len),
                self.count.get(len),
                self.start.get(len),
            ) else {
                break;
            };
            if code >= first && code - first < count {
                return self
                    .symbols
                    .get(start + (code - first) as usize)
                    .copied()
                    .ok_or(InflateError::Invalid("a code without a symbol"));
            }
        }
        Err(InflateError::Invalid("a code that is not in the table"))
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order in which a dynamic block gives the code lengths of the
/// code-length alphabet.
const CLEN_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn push(out: &mut Vec<u8>, byte: u8, limit: usize) -> Result<(), InflateError> {
    if out.len() >= limit {
        return Err(InflateError::TooLong);
    }
    out.push(byte);
    Ok(())
}

fn codes(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    limit: usize,
    lengths: &Huffman,
    distances: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = lengths.decode(bits)?;
        if symbol < 256 {
            push(out, symbol as u8, limit)?;
            continue;
        }
        if symbol == 256 {
            return Ok(());
        }
        let k = usize::from(symbol - 257);
        let (Some(&base), Some(&extra)) = (LENGTH_BASE.get(k), LENGTH_EXTRA.get(k)) else {
            return Err(InflateError::Invalid("a length symbol above 285"));
        };
        let length = usize::from(base) + bits.bits(u32::from(extra))? as usize;
        let d = usize::from(distances.decode(bits)?);
        let (Some(&base), Some(&extra)) = (DIST_BASE.get(d), DIST_EXTRA.get(d)) else {
            return Err(InflateError::Invalid("a distance symbol above 29"));
        };
        let distance = usize::from(base) + bits.bits(u32::from(extra))? as usize;
        if distance > out.len() {
            return Err(InflateError::Invalid("a distance back past the start"));
        }
        if out.len().saturating_add(length) > limit {
            return Err(InflateError::TooLong);
        }
        let from = out.len() - distance;
        for i in 0..length {
            let byte = out
                .get(from + i)
                .copied()
                .ok_or(InflateError::Invalid("a distance"))?;
            out.push(byte);
        }
    }
}

fn fixed() -> Result<(Huffman, Huffman), InflateError> {
    let mut lengths = [0u8; 288];
    for (i, l) in lengths.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), InflateError> {
    let nlen = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(InflateError::Invalid("too many length or distance codes"));
    }
    let mut clen = [0u8; 19];
    for &at in CLEN_ORDER.iter().take(ncode) {
        if let Some(slot) = clen.get_mut(at) {
            *slot = bits.bits(3)? as u8;
        }
    }
    let clen = Huffman::new(&clen)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut at = 0;
    while at < nlen + ndist {
        let symbol = clen.decode(bits)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                let previous = at
                    .checked_sub(1)
                    .and_then(|p| lengths.get(p))
                    .copied()
                    .ok_or(InflateError::Invalid("a repeat with nothing before it"))?;
                (previous, 3 + bits.bits(2)? as usize)
            }
            17 => (0, 3 + bits.bits(3)? as usize),
            18 => (0, 11 + bits.bits(7)? as usize),
            _ => return Err(InflateError::Invalid("a code-length symbol above 18")),
        };
        if at + repeat > nlen + ndist {
            return Err(InflateError::Invalid("code lengths past their count"));
        }
        for slot in lengths.iter_mut().skip(at).take(repeat) {
            *slot = value;
        }
        at += repeat;
    }
    if lengths.get(256).copied().unwrap_or(0) == 0 {
        return Err(InflateError::Invalid("no code for the end of a block"));
    }
    let (literal, distance) = lengths.split_at(nlen);
    Ok((Huffman::new(literal)?, Huffman::new(distance)?))
}

/// Decompresses a raw DEFLATE stream (no zlib or gzip wrapper) into at
/// most `limit` bytes.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    let mut bits = Bits {
        data,
        at: 0,
        buffer: 0,
        count: 0,
    };
    // A compressed stream expands at most 1032 times; the limit caps it
    // further, and the reservation stays small for a hostile size.
    let mut out = Vec::with_capacity(limit.min(data.len().saturating_mul(4)).min(1 << 24));
    loop {
        let last = bits.bit()?;
        match bits.bits(2)? {
            0 => {
                bits.align();
                let header = data
                    .get(bits.at..bits.at.saturating_add(4))
                    .ok_or(InflateError::Truncated)?;
                let (Some(&a), Some(&b), Some(&c), Some(&d)) =
                    (header.first(), header.get(1), header.get(2), header.get(3))
                else {
                    return Err(InflateError::Truncated);
                };
                let len = u16::from_le_bytes([a, b]);
                let nlen = u16::from_le_bytes([c, d]);
                if len != !nlen {
                    return Err(InflateError::Invalid("a stored block's length check"));
                }
                bits.at += 4;
                let len = usize::from(len);
                let block = data
                    .get(bits.at..bits.at.saturating_add(len))
                    .ok_or(InflateError::Truncated)?;
                if out.len().saturating_add(len) > limit {
                    return Err(InflateError::TooLong);
                }
                out.extend_from_slice(block);
                bits.at += len;
            }
            1 => {
                let (l, d) = fixed()?;
                codes(&mut bits, &mut out, limit, &l, &d)?;
            }
            2 => {
                let (l, d) = dynamic(&mut bits)?;
                codes(&mut bits, &mut out, limit, &l, &d)?;
            }
            _ => return Err(InflateError::Invalid("a block of the reserved type 3")),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}
