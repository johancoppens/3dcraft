//! The reader: the bytes of an exchange file as a [`StepFile`].

use crate::model::{Entity, Instance, Param, Record, StepFile};
use crate::write::{MAX_DEPTH, valid_binary, valid_keyword, valid_name};
use std::collections::BTreeSet;
use std::fmt;

/// Caps on what a file may ask of the reader. Everything the reader
/// allocates is proportional to the bytes it has read, so these are about
/// refusing early and bounding recursion, not about memory safety.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    /// Bytes of input.
    pub max_bytes: usize,
    /// Entity instances in the data section.
    pub max_instances: usize,
    /// Parameters in the whole file, counting every list item.
    pub max_params: usize,
    /// Nesting of lists and typed parameters; at most 64.
    pub max_depth: usize,
    /// Bytes of one string or binary token.
    pub max_string: usize,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_instances: 20_000_000,
            max_params: 400_000_000,
            max_depth: MAX_DEPTH,
            max_string: 1 << 24,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadErrorKind {
    /// The input is larger than [`ReadLimits::max_bytes`].
    TooLarge,
    /// The input ends where more was expected.
    UnexpectedEnd {
        expected: &'static str,
    },
    /// Something else stands where `expected` should.
    Unexpected {
        expected: &'static str,
    },
    /// A character that starts no token.
    BadCharacter,
    /// A comment that is not closed.
    OpenComment,
    /// A string or binary that is not closed, or too long.
    BadString {
        reason: &'static str,
    },
    /// A number that is malformed or does not fit (an integer beyond 64
    /// bits, a real beyond the range of a double).
    BadNumber {
        reason: &'static str,
    },
    BadKeyword,
    BadEnum,
    /// `#0`, or an instance name beyond 64 bits.
    BadInstanceName,
    DuplicateInstance {
        id: u64,
    },
    /// An instance refers to one the file does not have.
    UnresolvedReference {
        from: u64,
        to: u64,
    },
    /// One of [`ReadLimits`] is exceeded.
    LimitExceeded {
        what: &'static str,
    },
    /// A part of the standard this reader does not cover.
    Unsupported {
        what: &'static str,
    },
}

/// What went wrong, and where: `offset` is the byte offset in the input,
/// `line` and `column` count from 1 (columns in bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub kind: ReadErrorKind,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "STEP file, line {} column {}: ", self.line, self.column)?;
        match &self.kind {
            ReadErrorKind::TooLarge => write!(f, "the file is larger than the reader accepts"),
            ReadErrorKind::UnexpectedEnd { expected } => {
                write!(f, "the file ends where {expected} was expected")
            }
            ReadErrorKind::Unexpected { expected } => write!(f, "expected {expected}"),
            ReadErrorKind::BadCharacter => write!(f, "a character that starts no token"),
            ReadErrorKind::OpenComment => write!(f, "a comment that is never closed"),
            ReadErrorKind::BadString { reason } => write!(f, "malformed string: {reason}"),
            ReadErrorKind::BadNumber { reason } => write!(f, "malformed number: {reason}"),
            ReadErrorKind::BadKeyword => write!(f, "malformed keyword"),
            ReadErrorKind::BadEnum => write!(f, "malformed enumeration value"),
            ReadErrorKind::BadInstanceName => write!(f, "malformed entity instance name"),
            ReadErrorKind::DuplicateInstance { id } => {
                write!(f, "instance #{id} is defined more than once")
            }
            ReadErrorKind::UnresolvedReference { from, to } => {
                write!(f, "instance #{from} refers to #{to}, which does not exist")
            }
            ReadErrorKind::LimitExceeded { what } => write!(f, "too many {what}"),
            ReadErrorKind::Unsupported { what } => write!(f, "not supported: {what}"),
        }
    }
}

impl std::error::Error for ReadError {}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A keyword-like word: letters, digits, `_`, `-`, an optional
    /// leading `!`.
    Word(String),
    Integer(i64),
    Real(f64),
    String(String),
    Enum(String),
    Binary(String),
    Name(u64),
    /// One of `( ) , ; = $ *`.
    Punct(u8),
    End,
}

struct Reader<'a> {
    input: &'a [u8],
    at: usize,
    limits: ReadLimits,
    params: usize,
}

type Fail = (ReadErrorKind, usize);

fn hex(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'A'..=b'F' => Some(u32::from(byte - b'A') + 10),
        b'a'..=b'f' => Some(u32::from(byte - b'a') + 10),
        _ => None,
    }
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.at).copied()
    }

    fn byte_at(&self, at: usize) -> Option<u8> {
        self.input.get(at).copied()
    }

    /// Skips white space and comments.
    fn skip(&mut self) -> Result<(), Fail> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n' | 0x0C) => self.at += 1,
                Some(b'/') if self.byte_at(self.at + 1) == Some(b'*') => {
                    let start = self.at;
                    let mut at = self.at + 2;
                    loop {
                        match (self.byte_at(at), self.byte_at(at + 1)) {
                            (Some(b'*'), Some(b'/')) => break,
                            (Some(_), _) => at += 1,
                            (None, _) => return Err((ReadErrorKind::OpenComment, start)),
                        }
                    }
                    self.at = at + 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn hex_run(&self, at: usize, digits: usize) -> Option<u32> {
        let mut value = 0_u32;
        for k in 0..digits {
            value = value.checked_mul(16)? + hex(self.byte_at(at.checked_add(k)?)?)?;
        }
        Some(value)
    }

    /// A string token; `self.at` is on the opening apostrophe.
    fn string(&mut self) -> Result<String, Fail> {
        let start = self.at;
        let bad = |reason: &'static str, at: usize| (ReadErrorKind::BadString { reason }, at);
        let mut at = self.at + 1;
        // The decoded characters. Raw bytes above 127 are collected as
        // they are and sorted out at the end.
        let mut out: Vec<u8> = Vec::new();
        let mut raw_high = false;
        let push = |out: &mut Vec<u8>, code: u32, at: usize| -> Result<(), Fail> {
            let c = char::from_u32(code).ok_or(bad("not a character code", at))?;
            let mut buffer = [0_u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
            Ok(())
        };
        // Whether the code page of `\S\` is ISO 8859-1.
        let mut latin1 = true;
        loop {
            if at.saturating_sub(start) > self.limits.max_string {
                return Err(bad("longer than the reader accepts", start));
            }
            let Some(byte) = self.byte_at(at) else {
                return Err(bad("never closed", start));
            };
            match byte {
                b'\'' => {
                    if self.byte_at(at + 1) == Some(b'\'') {
                        out.push(b'\'');
                        at += 2;
                    } else {
                        at += 1;
                        break;
                    }
                }
                b'\\' => match (self.byte_at(at + 1), self.byte_at(at + 2)) {
                    (Some(b'\\'), _) => {
                        out.push(b'\\');
                        at += 2;
                    }
                    (Some(b'S'), Some(b'\\')) => {
                        let c = self.byte_at(at + 3).ok_or(bad("never closed", start))?;
                        if !(0x20..0x7F).contains(&c) {
                            return Err(bad("\\S\\ needs a printable character", at));
                        }
                        let code = if latin1 { u32::from(c) + 128 } else { 0xFFFD };
                        push(&mut out, code, at)?;
                        at += 4;
                    }
                    (Some(b'P'), Some(page @ b'A'..=b'I'))
                        if self.byte_at(at + 3) == Some(b'\\') =>
                    {
                        latin1 = page == b'A';
                        at += 4;
                    }
                    (Some(b'X'), Some(b'\\')) => {
                        let code = self
                            .hex_run(at + 3, 2)
                            .ok_or(bad("\\X\\ needs two hexadecimal digits", at))?;
                        push(&mut out, code, at)?;
                        at += 5;
                    }
                    (Some(b'X'), Some(width @ (b'2' | b'4')))
                        if self.byte_at(at + 3) == Some(b'\\') =>
                    {
                        let digits = if width == b'2' { 4 } else { 8 };
                        let directive = at;
                        at += 4;
                        // A UTF-16 high surrogate waiting for its low one.
                        let mut high: Option<u32> = None;
                        loop {
                            if self.byte_at(at) == Some(b'\\') {
                                if self.byte_at(at + 1) == Some(b'X')
                                    && self.byte_at(at + 2) == Some(b'0')
                                    && self.byte_at(at + 3) == Some(b'\\')
                                {
                                    at += 4;
                                    break;
                                }
                                return Err(bad(
                                    "\\X2\\ or \\X4\\ not closed by \\X0\\",
                                    directive,
                                ));
                            }
                            if at.saturating_sub(start) > self.limits.max_string {
                                return Err(bad("longer than the reader accepts", start));
                            }
                            let code = self.hex_run(at, digits).ok_or(bad(
                                "\\X2\\ needs groups of four and \\X4\\ of eight hexadecimal digits, closed by \\X0\\",
                                at,
                            ))?;
                            at += digits;
                            match (high.take(), code) {
                                (Some(h), 0xDC00..=0xDFFF) if digits == 4 => {
                                    push(
                                        &mut out,
                                        0x10000 + ((h - 0xD800) << 10) + (code - 0xDC00),
                                        at,
                                    )?;
                                }
                                (Some(_), _) => return Err(bad("unpaired surrogate", at)),
                                (None, 0xD800..=0xDBFF) if digits == 4 => high = Some(code),
                                (None, _) => push(&mut out, code, at)?,
                            }
                        }
                        if high.is_some() {
                            return Err(bad("unpaired surrogate", at));
                        }
                    }
                    _ => return Err(bad("unknown control directive after a backslash", at)),
                },
                0x20..=0x7E => {
                    out.push(byte);
                    at += 1;
                }
                0x80..=0xFF => {
                    raw_high = true;
                    out.push(byte);
                    at += 1;
                }
                // Line breaks inside a string are not part of it
                // (edition 1 files were wrapped at 80 columns).
                b'\r' | b'\n' => at += 1,
                _ => return Err(bad("a control character", at)),
            }
        }
        self.at = at;
        match String::from_utf8(out) {
            Ok(text) => Ok(text),
            // Not UTF-8: the raw bytes are ISO 8859-1. Only possible when
            // there were raw high bytes; what the directives decoded is
            // ASCII or went in as UTF-8 and is taken apart again here,
            // which is the best that can be made of such a file.
            Err(error) if raw_high => Ok(error.into_bytes().into_iter().map(char::from).collect()),
            Err(_) => Err(bad("not text", start)),
        }
    }

    /// A number token; `self.at` is on its sign or first digit.
    fn number(&mut self) -> Result<Token, Fail> {
        let start = self.at;
        let bad = |reason: &'static str| (ReadErrorKind::BadNumber { reason }, start);
        let mut at = self.at;
        if matches!(self.byte_at(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let digits_from = at;
        while matches!(self.byte_at(at), Some(b'0'..=b'9')) {
            at += 1;
        }
        if at == digits_from {
            return Err(bad("a sign without digits"));
        }
        let mut real = false;
        if self.byte_at(at) == Some(b'.') {
            real = true;
            at += 1;
            while matches!(self.byte_at(at), Some(b'0'..=b'9')) {
                at += 1;
            }
            if matches!(self.byte_at(at), Some(b'E' | b'e')) {
                at += 1;
                if matches!(self.byte_at(at), Some(b'+' | b'-')) {
                    at += 1;
                }
                let exponent_from = at;
                while matches!(self.byte_at(at), Some(b'0'..=b'9')) {
                    at += 1;
                }
                if at == exponent_from {
                    return Err(bad("an exponent without digits"));
                }
            }
        }
        if at - start > 400 {
            return Err(bad("too many digits"));
        }
        let text = self
            .input
            .get(start..at)
            .and_then(|b| std::str::from_utf8(b).ok())
            .ok_or(bad("not a number"))?;
        self.at = at;
        if real {
            // Rust wants a digit after the point before an exponent.
            let text = text.replacen(".E", ".0E", 1).replacen(".e", ".0e", 1);
            let value: f64 = text.parse().map_err(|_| bad("not a real"))?;
            if !value.is_finite() {
                return Err(bad("beyond the range of a double"));
            }
            Ok(Token::Real(value))
        } else {
            let text = text.strip_prefix('+').unwrap_or(text);
            text.parse()
                .map(Token::Integer)
                .map_err(|_| bad("an integer beyond 64 bits"))
        }
    }

    /// The next token and the offset it starts at.
    fn token(&mut self) -> Result<(Token, usize), Fail> {
        self.skip()?;
        let start = self.at;
        let Some(byte) = self.peek() else {
            return Ok((Token::End, start));
        };
        let token = match byte {
            b'(' | b')' | b',' | b';' | b'=' | b'$' | b'*' => {
                self.at += 1;
                Token::Punct(byte)
            }
            b'\'' => Token::String(self.string()?),
            b'"' => {
                let mut at = self.at + 1;
                while matches!(self.byte_at(at), Some(b) if b != b'"') {
                    if at - start > self.limits.max_string {
                        return Err((
                            ReadErrorKind::BadString {
                                reason: "longer than the reader accepts",
                            },
                            start,
                        ));
                    }
                    at += 1;
                }
                let text = self
                    .input
                    .get(start + 1..at)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .filter(|t| self.byte_at(at) == Some(b'"') && valid_binary(t))
                    .ok_or((
                        ReadErrorKind::BadString {
                            reason: "a binary must be hexadecimal digits between double quotes",
                        },
                        start,
                    ))?;
                self.at = at + 1;
                Token::Binary(text.to_string())
            }
            b'#' => {
                let mut at = self.at + 1;
                let mut id = 0_u64;
                while let Some(d @ b'0'..=b'9') = self.byte_at(at) {
                    id = id
                        .checked_mul(10)
                        .and_then(|v| v.checked_add(u64::from(d - b'0')))
                        .ok_or((ReadErrorKind::BadInstanceName, start))?;
                    at += 1;
                }
                if at == self.at + 1 || id == 0 {
                    return Err((ReadErrorKind::BadInstanceName, start));
                }
                self.at = at;
                Token::Name(id)
            }
            b'.' => {
                let mut at = self.at + 1;
                while matches!(self.byte_at(at), Some(b) if b.is_ascii_alphanumeric() || b == b'_')
                {
                    at += 1;
                }
                let name = self
                    .input
                    .get(start + 1..at)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .filter(|n| self.byte_at(at) == Some(b'.') && valid_name(n))
                    .ok_or((ReadErrorKind::BadEnum, start))?;
                self.at = at + 1;
                Token::Enum(name.to_string())
            }
            b'+' | b'-' | b'0'..=b'9' => self.number()?,
            b'&' => {
                return Err((
                    ReadErrorKind::Unsupported {
                        what: "the &SCOPE construct of edition 1",
                    },
                    start,
                ));
            }
            b'!' | b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let mut at = self.at + 1;
                while matches!(self.byte_at(at), Some(b) if b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    at += 1;
                }
                let word = self
                    .input
                    .get(start..at)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .ok_or((ReadErrorKind::BadKeyword, start))?;
                self.at = at;
                Token::Word(word.to_string())
            }
            _ => return Err((ReadErrorKind::BadCharacter, start)),
        };
        Ok((token, start))
    }

    fn expect_punct(&mut self, punct: u8, expected: &'static str) -> Result<(), Fail> {
        match self.token()? {
            (Token::Punct(p), _) if p == punct => Ok(()),
            (Token::End, at) => Err((ReadErrorKind::UnexpectedEnd { expected }, at)),
            (_, at) => Err((ReadErrorKind::Unexpected { expected }, at)),
        }
    }

    /// `WORD;` exactly.
    fn expect_word(&mut self, word: &str, expected: &'static str) -> Result<(), Fail> {
        match self.token()? {
            (Token::Word(w), _) if w == word => self.expect_punct(b';', "`;`"),
            (Token::End, at) => Err((ReadErrorKind::UnexpectedEnd { expected }, at)),
            (_, at) => Err((ReadErrorKind::Unexpected { expected }, at)),
        }
    }

    fn count_param(&mut self, at: usize) -> Result<(), Fail> {
        self.params += 1;
        if self.params > self.limits.max_params {
            return Err((ReadErrorKind::LimitExceeded { what: "parameters" }, at));
        }
        Ok(())
    }

    /// The parameters of a list whose `(` has been read, up to and
    /// including its `)`.
    fn list(&mut self, depth: usize) -> Result<Vec<Param>, Fail> {
        let mut items = Vec::new();
        let (first, at) = self.token()?;
        if first == Token::Punct(b')') {
            return Ok(items);
        }
        items.push(self.param(first, at, depth)?);
        loop {
            match self.token()? {
                (Token::Punct(b','), _) => {
                    let (token, at) = self.token()?;
                    items.push(self.param(token, at, depth)?);
                }
                (Token::Punct(b')'), _) => return Ok(items),
                (Token::End, at) => {
                    return Err((
                        ReadErrorKind::UnexpectedEnd {
                            expected: "`,` or `)`",
                        },
                        at,
                    ));
                }
                (_, at) => {
                    return Err((
                        ReadErrorKind::Unexpected {
                            expected: "`,` or `)`",
                        },
                        at,
                    ));
                }
            }
        }
    }

    /// The parameter that starts with `token`. `depth` is how deep it is
    /// nested: the parameters of a record are at depth 1.
    fn param(&mut self, token: Token, at: usize, depth: usize) -> Result<Param, Fail> {
        self.count_param(at)?;
        let too_deep = (
            ReadErrorKind::LimitExceeded {
                what: "levels of nesting",
            },
            at,
        );
        Ok(match token {
            Token::Punct(b'$') => Param::Omitted,
            Token::Punct(b'*') => Param::Derived,
            Token::Integer(i) => Param::Integer(i),
            Token::Real(x) => Param::Real(x),
            Token::String(s) => Param::String(s),
            Token::Enum(name) => Param::Enum(name),
            Token::Binary(bits) => Param::Binary(bits),
            Token::Name(id) => Param::Ref(id),
            Token::Punct(b'(') => {
                if depth >= self.limits.max_depth.min(MAX_DEPTH) {
                    return Err(too_deep);
                }
                Param::List(self.list(depth + 1)?)
            }
            Token::Word(keyword) => {
                if !valid_keyword(&keyword) {
                    return Err((ReadErrorKind::BadKeyword, at));
                }
                if depth >= self.limits.max_depth.min(MAX_DEPTH) {
                    return Err(too_deep);
                }
                self.expect_punct(b'(', "`(` after the keyword of a typed parameter")?;
                let (token, inner_at) = self.token()?;
                let inner = self.param(token, inner_at, depth + 1)?;
                self.expect_punct(b')', "`)`: a typed parameter has one value")?;
                Param::Typed(keyword, Box::new(inner))
            }
            Token::End => {
                return Err((
                    ReadErrorKind::UnexpectedEnd {
                        expected: "a parameter",
                    },
                    at,
                ));
            }
            Token::Punct(_) => {
                return Err((
                    ReadErrorKind::Unexpected {
                        expected: "a parameter",
                    },
                    at,
                ));
            }
        })
    }

    /// `KEYWORD(params)`, the keyword already read.
    fn record(&mut self, keyword: String, at: usize) -> Result<Record, Fail> {
        if !valid_keyword(&keyword) {
            return Err((ReadErrorKind::BadKeyword, at));
        }
        self.expect_punct(b'(', "`(` after a keyword")?;
        Ok(Record {
            keyword,
            params: self.list(1)?,
        })
    }

    fn file(&mut self) -> Result<StepFile, Fail> {
        self.expect_word("ISO-10303-21", "`ISO-10303-21;` at the start of the file")?;
        self.expect_word("HEADER", "`HEADER;`")?;
        let mut file = StepFile::default();
        loop {
            match self.token()? {
                (Token::Word(word), _) if word == "ENDSEC" => {
                    self.expect_punct(b';', "`;`")?;
                    break;
                }
                (Token::Word(keyword), at) => {
                    if file.header.len() >= 10_000 {
                        return Err((
                            ReadErrorKind::LimitExceeded {
                                what: "header records",
                            },
                            at,
                        ));
                    }
                    file.header.push(self.record(keyword, at)?);
                    self.expect_punct(b';', "`;` after a header record")?;
                }
                (Token::End, at) => {
                    return Err((
                        ReadErrorKind::UnexpectedEnd {
                            expected: "a header record or `ENDSEC;`",
                        },
                        at,
                    ));
                }
                (_, at) => {
                    return Err((
                        ReadErrorKind::Unexpected {
                            expected: "a header record or `ENDSEC;`",
                        },
                        at,
                    ));
                }
            }
        }
        match self.token()? {
            (Token::Word(word), _) if word == "DATA" => {}
            (Token::Word(word), at)
                if matches!(word.as_str(), "ANCHOR" | "REFERENCE" | "SIGNATURE") =>
            {
                return Err((
                    ReadErrorKind::Unsupported {
                        what: "the ANCHOR, REFERENCE and SIGNATURE sections of edition 3",
                    },
                    at,
                ));
            }
            (Token::End, at) => {
                return Err((
                    ReadErrorKind::UnexpectedEnd {
                        expected: "`DATA;`",
                    },
                    at,
                ));
            }
            (_, at) => {
                return Err((
                    ReadErrorKind::Unexpected {
                        expected: "`DATA;`",
                    },
                    at,
                ));
            }
        }
        // Edition 3 lets a data section carry a name and its schema:
        // `DATA('name', ('schema'));`. Read and dropped.
        match self.token()? {
            (Token::Punct(b';'), _) => {}
            (Token::Punct(b'('), _) => {
                self.list(1)?;
                self.expect_punct(b';', "`;`")?;
            }
            (Token::End, at) => {
                return Err((ReadErrorKind::UnexpectedEnd { expected: "`;`" }, at));
            }
            (_, at) => return Err((ReadErrorKind::Unexpected { expected: "`;`" }, at)),
        }
        let mut ids = BTreeSet::new();
        // Where each instance starts, for the errors found afterwards.
        let mut offsets = Vec::new();
        loop {
            match self.token()? {
                (Token::Word(word), _) if word == "ENDSEC" => {
                    self.expect_punct(b';', "`;`")?;
                    break;
                }
                (Token::Name(id), at) => {
                    if file.data.len() >= self.limits.max_instances {
                        return Err((ReadErrorKind::LimitExceeded { what: "instances" }, at));
                    }
                    if !ids.insert(id) {
                        return Err((ReadErrorKind::DuplicateInstance { id }, at));
                    }
                    self.expect_punct(b'=', "`=` after an instance name")?;
                    let entity = match self.token()? {
                        (Token::Word(keyword), at) => Entity::Simple(self.record(keyword, at)?),
                        (Token::Punct(b'('), open) => {
                            let mut records = Vec::new();
                            loop {
                                match self.token()? {
                                    (Token::Word(keyword), at) => {
                                        records.push(self.record(keyword, at)?);
                                    }
                                    (Token::Punct(b')'), at) => {
                                        if records.is_empty() {
                                            return Err((
                                                ReadErrorKind::Unexpected {
                                                    expected: "a record of the complex instance",
                                                },
                                                at,
                                            ));
                                        }
                                        break;
                                    }
                                    (Token::End, _) => {
                                        return Err((
                                            ReadErrorKind::UnexpectedEnd {
                                                expected: "`)` to close the complex instance",
                                            },
                                            open,
                                        ));
                                    }
                                    (_, at) => {
                                        return Err((
                                            ReadErrorKind::Unexpected {
                                                expected: "a record of the complex instance or `)`",
                                            },
                                            at,
                                        ));
                                    }
                                }
                            }
                            Entity::Complex(records)
                        }
                        (Token::End, at) => {
                            return Err((
                                ReadErrorKind::UnexpectedEnd {
                                    expected: "an entity after `=`",
                                },
                                at,
                            ));
                        }
                        (_, at) => {
                            return Err((
                                ReadErrorKind::Unexpected {
                                    expected: "an entity after `=`",
                                },
                                at,
                            ));
                        }
                    };
                    self.expect_punct(b';', "`;` after an instance")?;
                    file.data.push(Instance { id, entity });
                    offsets.push(at);
                }
                (Token::End, at) => {
                    return Err((
                        ReadErrorKind::UnexpectedEnd {
                            expected: "an instance or `ENDSEC;`",
                        },
                        at,
                    ));
                }
                (_, at) => {
                    return Err((
                        ReadErrorKind::Unexpected {
                            expected: "an instance (`#id = …;`) or `ENDSEC;`",
                        },
                        at,
                    ));
                }
            }
        }
        match self.token()? {
            (Token::Word(word), _) if word == "END-ISO-10303-21" => {
                self.expect_punct(b';', "`;`")?;
            }
            (Token::Word(word), at) if word == "DATA" => {
                return Err((
                    ReadErrorKind::Unsupported {
                        what: "more than one data section",
                    },
                    at,
                ));
            }
            (Token::End, at) => {
                return Err((
                    ReadErrorKind::UnexpectedEnd {
                        expected: "`END-ISO-10303-21;`",
                    },
                    at,
                ));
            }
            (_, at) => {
                return Err((
                    ReadErrorKind::Unexpected {
                        expected: "`END-ISO-10303-21;`",
                    },
                    at,
                ));
            }
        }
        // Anything after the end is not ours to judge (some writers add a
        // line or a Ctrl-Z); references must resolve, though.
        for (instance, &at) in file.data.iter().zip(&offsets) {
            let mut missing = None;
            instance.entity.for_each_ref(&mut |to| {
                if missing.is_none() && !ids.contains(&to) {
                    missing = Some(to);
                }
            });
            if let Some(to) = missing {
                return Err((
                    ReadErrorKind::UnresolvedReference {
                        from: instance.id,
                        to,
                    },
                    at,
                ));
            }
        }
        Ok(file)
    }
}

/// Reads an exchange file with the default [`ReadLimits`].
pub fn read(input: &[u8]) -> Result<StepFile, ReadError> {
    read_with(input, &ReadLimits::default())
}

/// Reads an exchange file.
///
/// The result has the header records and the instances in file order,
/// each id once, and every reference resolves; whether the references form
/// a cycle is for [`StepFile::reference_cycle`] to say.
pub fn read_with(input: &[u8], limits: &ReadLimits) -> Result<StepFile, ReadError> {
    let located = |(kind, offset): Fail| {
        let before = input.get(..offset.min(input.len())).unwrap_or(&[]);
        let line = 1 + before.iter().filter(|&&b| b == b'\n').count();
        let line_start = before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        ReadError {
            kind,
            offset,
            line,
            column: 1 + offset.saturating_sub(line_start),
        }
    };
    if input.len() > limits.max_bytes {
        return Err(located((ReadErrorKind::TooLarge, 0)));
    }
    let mut reader = Reader {
        input,
        at: 0,
        limits: *limits,
        params: 0,
    };
    reader.file().map_err(located)
}
