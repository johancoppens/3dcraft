//! Reading request lines with a size cap, and the serve loop both protocols
//! share.

use crate::MAX_REQUEST_BYTES;
use craft3d_engine::Session;
use std::io::{self, BufRead, ErrorKind, Write};

/// What one read from the input produced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// The input has ended.
    End,
    /// A line, without its line ending.
    Bytes(Vec<u8>),
    /// A line longer than the cap. It has been consumed up to and including
    /// its newline, and none of it was kept.
    TooLong,
}

/// Reads one line of at most `cap` bytes.
///
/// `BufRead::read_line` would buffer a whole line first, so a peer that
/// never sends a newline could exhaust memory. This keeps at most `cap`
/// bytes and discards the rest of an over-long line as it streams past.
pub(crate) fn read_line(input: &mut impl BufRead, cap: usize) -> io::Result<Line> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut read_any = false;
    loop {
        let buffer = match input.fill_buf() {
            Ok(buffer) => buffer,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buffer.is_empty() {
            if !read_any {
                return Ok(Line::End);
            }
            // The last line of the input has no newline: still a line.
            break;
        }
        read_any = true;
        let newline = buffer.iter().position(|&byte| byte == b'\n');
        let chunk = match newline {
            Some(at) => buffer.get(..at).unwrap_or(buffer),
            None => buffer,
        };
        if !too_long {
            if line.len().saturating_add(chunk.len()) > cap {
                too_long = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        let used = newline.map_or(buffer.len(), |at| at.saturating_add(1));
        input.consume(used);
        if newline.is_some() {
            break;
        }
    }
    if too_long {
        return Ok(Line::TooLong);
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Line::Bytes(line))
}

/// Why a line never reached a protocol's handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejected {
    TooLong,
    NotUtf8,
}

impl Rejected {
    pub(crate) fn message(self) -> String {
        match self {
            Rejected::TooLong => too_long_message(),
            Rejected::NotUtf8 => "the request line is not valid UTF-8".to_string(),
        }
    }
}

pub(crate) fn too_long_message() -> String {
    format!(
        "the request line is longer than the limit of {MAX_REQUEST_BYTES} bytes; send smaller requests"
    )
}

/// The two functions that make a protocol.
pub(crate) struct Protocol {
    /// One request line in, at most one reply line out.
    pub handle: fn(&mut Session, &str) -> Option<String>,
    /// The reply to a line that could not be handed to `handle`.
    pub reject: fn(Rejected) -> String,
}

/// Reads requests until the input ends, writing and flushing each reply
/// before reading the next request. Only an I/O error ends it early.
pub(crate) fn serve(
    session: &mut Session,
    mut input: impl BufRead,
    mut output: impl Write,
    protocol: &Protocol,
) -> io::Result<()> {
    loop {
        let reply = match read_line(&mut input, MAX_REQUEST_BYTES)? {
            Line::End => return Ok(()),
            Line::TooLong => Some((protocol.reject)(Rejected::TooLong)),
            Line::Bytes(bytes) => match std::str::from_utf8(&bytes) {
                Ok(line) => (protocol.handle)(session, line),
                Err(_) => Some((protocol.reject)(Rejected::NotUtf8)),
            },
        };
        if let Some(reply) = reply {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn lines(input: &[u8], cap: usize, buffer: usize) -> Vec<Line> {
        let mut reader = BufReader::with_capacity(buffer, Cursor::new(input.to_vec()));
        let mut out = Vec::new();
        loop {
            match read_line(&mut reader, cap).unwrap() {
                Line::End => return out,
                line => out.push(line),
            }
        }
    }

    fn text(s: &str) -> Line {
        Line::Bytes(s.as_bytes().to_vec())
    }

    #[test]
    fn splits_on_newlines_and_strips_carriage_returns() {
        // A tiny buffer forces lines to be assembled from several reads.
        for buffer in [1, 2, 3, 64] {
            assert_eq!(
                lines(b"one\ntwo\r\n\nlast", 100, buffer),
                vec![text("one"), text("two"), text(""), text("last")],
                "buffer {buffer}"
            );
        }
        assert_eq!(lines(b"", 100, 8), vec![]);
        assert_eq!(lines(b"\n", 100, 8), vec![text("")]);
    }

    #[test]
    fn an_over_long_line_is_dropped_and_the_next_line_is_still_read() {
        for buffer in [1, 4, 64] {
            assert_eq!(
                lines(b"12345\n123456\nok\n1234567", 5, buffer),
                vec![text("12345"), Line::TooLong, text("ok"), Line::TooLong],
                "buffer {buffer}"
            );
        }
    }

    /// A reader that would produce bytes for ever and never a newline.
    struct Endless {
        left: usize,
    }

    impl io::Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let n = buffer.len().min(self.left);
            buffer.iter_mut().take(n).for_each(|b| *b = b'x');
            self.left -= n;
            Ok(n)
        }
    }

    #[test]
    fn a_line_without_a_newline_does_not_grow_the_buffer_past_the_cap() {
        // 64 MiB streams past with a 1 KiB cap; the test would take
        // gigabytes if the line were kept.
        let mut reader = BufReader::new(Endless { left: 64 << 20 });
        assert_eq!(read_line(&mut reader, 1024).unwrap(), Line::TooLong);
        assert_eq!(read_line(&mut reader, 1024).unwrap(), Line::End);
    }
}
