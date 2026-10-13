//! The writer: a [`StepFile`] as the text of an exchange file.

use crate::model::{Entity, Param, Record, StepFile};
use std::collections::BTreeSet;
use std::fmt;

/// Parameters nested deeper than this are refused, on write as on read.
pub(crate) const MAX_DEPTH: usize = 64;
/// A line is broken at the first comma past this column. Tokens are never
/// split, so a long string can still make a longer line.
const WRAP: usize = 78;

#[derive(Clone, Debug, PartialEq)]
pub enum WriteError {
    /// NaN or infinity: Part 21 has no text for them.
    NonFiniteReal { instance: u64 },
    /// A keyword that is not upper-case letters, digits and underscores
    /// starting with a letter (after an optional `!`), or an empty one.
    BadKeyword { instance: u64, keyword: String },
    /// An enumeration value that is not upper-case letters, digits and
    /// underscores starting with a letter.
    BadEnum { instance: u64, value: String },
    /// A binary that is not hexadecimal digits with a first digit 0 to 3.
    BadBinary { instance: u64 },
    /// An instance with id 0, or two instances with the same id.
    BadId { instance: u64 },
    /// A complex instance without records.
    EmptyComplex { instance: u64 },
    /// Parameters nested deeper than the reader accepts.
    TooDeep { instance: u64 },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Instance 0 stands for the header.
        let at = |instance: &u64| {
            if *instance == 0 {
                "the header".to_string()
            } else {
                format!("instance #{instance}")
            }
        };
        match self {
            WriteError::NonFiniteReal { instance } => write!(
                f,
                "{} has a real that is not finite (NaN or infinity)",
                at(instance)
            ),
            WriteError::BadKeyword { instance, keyword } => {
                write!(f, "{}: `{keyword}` is not a valid keyword", at(instance))
            }
            WriteError::BadEnum { instance, value } => write!(
                f,
                "{}: `{value}` is not a valid enumeration value",
                at(instance)
            ),
            WriteError::BadBinary { instance } => {
                write!(f, "{} has a malformed binary value", at(instance))
            }
            WriteError::BadId { instance } => {
                write!(f, "instance id #{instance} is zero or used more than once")
            }
            WriteError::EmptyComplex { instance } => {
                write!(f, "{} is a complex instance without records", at(instance))
            }
            WriteError::TooDeep { instance } => write!(
                f,
                "{} has parameters nested deeper than {MAX_DEPTH} levels",
                at(instance)
            ),
        }
    }
}

impl std::error::Error for WriteError {}

/// The text of a real: the shortest decimal that reads back as exactly
/// `x`, in the form Part 21 asks for (digits, a point, optional digits,
/// optional `E` and exponent). `None` for NaN and infinity.
pub fn format_real(x: f64) -> Option<String> {
    if !x.is_finite() {
        return None;
    }
    // Rust's `{:?}` is the shortest text that round-trips; it has either a
    // point or an exponent, or both.
    let shortest = format!("{x:?}");
    let (mantissa, exponent) = match shortest.split_once('e') {
        Some((m, e)) => (m, Some(e)),
        None => (shortest.as_str(), None),
    };
    let mut out = String::with_capacity(shortest.len() + 2);
    match mantissa.strip_suffix(".0") {
        Some(whole) => {
            out.push_str(whole);
            out.push('.');
        }
        None => {
            out.push_str(mantissa);
            if !mantissa.contains('.') {
                out.push('.');
            }
        }
    }
    if let Some(exponent) = exponent {
        out.push('E');
        out.push_str(exponent);
    }
    Some(out)
}

pub(crate) fn valid_keyword(keyword: &str) -> bool {
    let name = keyword.strip_prefix('!').unwrap_or(keyword);
    valid_name(name)
}

pub(crate) fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'_'))
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

pub(crate) fn valid_binary(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(b'0'..=b'3'))
        && bytes.all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

fn push_string(out: &mut String, text: &str) {
    out.push('\'');
    // 0 = plain, 2 = inside \X2\, 4 = inside \X4\.
    let mut mode = 0_u8;
    let close = |out: &mut String, mode: &mut u8| {
        if *mode != 0 {
            out.push_str("\\X0\\");
            *mode = 0;
        }
    };
    for c in text.chars() {
        let code = u32::from(c);
        if (0x20..0x7F).contains(&code) {
            close(out, &mut mode);
            match c {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                _ => out.push(c),
            }
        } else if code <= 0xFFFF {
            if mode != 2 {
                close(out, &mut mode);
                out.push_str("\\X2\\");
                mode = 2;
            }
            out.push_str(&format!("{code:04X}"));
        } else {
            if mode != 4 {
                close(out, &mut mode);
                out.push_str("\\X4\\");
                mode = 4;
            }
            out.push_str(&format!("{code:08X}"));
        }
    }
    close(out, &mut mode);
    out.push('\'');
}

/// Text under construction, with the column of its last line.
struct Out {
    text: String,
    line_start: usize,
}

impl Out {
    /// A comma, and a line break after it when the line is long.
    fn comma(&mut self) {
        self.text.push(',');
        if self.text.len().saturating_sub(self.line_start) > WRAP {
            self.text.push('\n');
            self.line_start = self.text.len();
            self.text.push_str("  ");
        }
    }

    fn end_line(&mut self) {
        self.text.push('\n');
        self.line_start = self.text.len();
    }
}

fn push_param(out: &mut Out, param: &Param, instance: u64, depth: usize) -> Result<(), WriteError> {
    if depth > MAX_DEPTH {
        return Err(WriteError::TooDeep { instance });
    }
    match param {
        Param::Omitted => out.text.push('$'),
        Param::Derived => out.text.push('*'),
        Param::Integer(i) => out.text.push_str(&i.to_string()),
        Param::Real(x) => out
            .text
            .push_str(&format_real(*x).ok_or(WriteError::NonFiniteReal { instance })?),
        Param::String(s) => push_string(&mut out.text, s),
        Param::Enum(name) => {
            if !valid_name(name) {
                return Err(WriteError::BadEnum {
                    instance,
                    value: name.clone(),
                });
            }
            out.text.push('.');
            out.text.push_str(name);
            out.text.push('.');
        }
        Param::Binary(bits) => {
            if !valid_binary(bits) {
                return Err(WriteError::BadBinary { instance });
            }
            out.text.push('"');
            out.text.push_str(bits);
            out.text.push('"');
        }
        Param::Ref(id) => {
            if *id == 0 {
                return Err(WriteError::BadId { instance });
            }
            out.text.push('#');
            out.text.push_str(&id.to_string());
        }
        Param::List(items) => {
            out.text.push('(');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.comma();
                }
                push_param(out, item, instance, depth + 1)?;
            }
            out.text.push(')');
        }
        Param::Typed(keyword, inner) => {
            if !valid_keyword(keyword) {
                return Err(WriteError::BadKeyword {
                    instance,
                    keyword: keyword.clone(),
                });
            }
            out.text.push_str(keyword);
            out.text.push('(');
            push_param(out, inner, instance, depth + 1)?;
            out.text.push(')');
        }
    }
    Ok(())
}

fn push_record(out: &mut Out, record: &Record, instance: u64) -> Result<(), WriteError> {
    if !valid_keyword(&record.keyword) {
        return Err(WriteError::BadKeyword {
            instance,
            keyword: record.keyword.clone(),
        });
    }
    out.text.push_str(&record.keyword);
    out.text.push('(');
    for (i, param) in record.params.iter().enumerate() {
        if i > 0 {
            out.comma();
        }
        push_param(out, param, instance, 1)?;
    }
    out.text.push(')');
    Ok(())
}

/// The exchange file as text (ASCII only).
///
/// Instances are written in the order of `file.data` with the ids they
/// have, so the same model always gives the same bytes.
pub fn write(file: &StepFile) -> Result<String, WriteError> {
    let mut out = Out {
        text: String::new(),
        line_start: 0,
    };
    out.text.push_str("ISO-10303-21;\nHEADER;\n");
    out.line_start = out.text.len();
    for record in &file.header {
        push_record(&mut out, record, 0)?;
        out.text.push(';');
        out.end_line();
    }
    out.text.push_str("ENDSEC;\nDATA;\n");
    out.line_start = out.text.len();
    let mut seen = BTreeSet::new();
    for instance in &file.data {
        let id = instance.id;
        if id == 0 || !seen.insert(id) {
            return Err(WriteError::BadId { instance: id });
        }
        out.text.push('#');
        out.text.push_str(&id.to_string());
        out.text.push('=');
        match &instance.entity {
            Entity::Simple(record) => push_record(&mut out, record, id)?,
            Entity::Complex(records) => {
                if records.is_empty() {
                    return Err(WriteError::EmptyComplex { instance: id });
                }
                out.text.push('(');
                for (i, record) in records.iter().enumerate() {
                    // A new line once the line is well filled.
                    if i > 0 && out.text.len().saturating_sub(out.line_start) > 48 {
                        out.end_line();
                        out.text.push_str("  ");
                    } else if i > 0 {
                        out.text.push(' ');
                    }
                    push_record(&mut out, record, id)?;
                }
                out.text.push(')');
            }
        }
        out.text.push(';');
        out.end_line();
    }
    out.text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    Ok(out.text)
}
