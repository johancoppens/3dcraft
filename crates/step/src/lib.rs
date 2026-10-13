//! ISO 10303-21 ("Part 21", the STEP physical file) reader and writer with
//! its own format-level model.
//!
//! Standalone: depends on no workspace crate (docs/architecture.md §3). It
//! knows the exchange structure and nothing about CAD: a file is a header
//! (a list of records) and one data section (a list of entity instances,
//! each a record or, for a complex instance, several), and a record is a
//! keyword with parameters. Which entities exist and what their parameters
//! mean is the business of whoever builds or interprets the instances
//! (`craft3d-io` for B-rep bodies).
//!
//! What is covered, by clause of ISO 10303-21 (edition 2, 2002; the
//! clause numbers are from memory of that edition and may be off by one in
//! another):
//!
//! - **Tokens (clause 6).** Integers, reals, strings, enumerations
//!   (`.NAME.`), binaries (`"0AF"`), entity instance names (`#12`),
//!   keywords (standard, and user-defined with a leading `!`), the omitted
//!   parameter `$` and the derived one `*`. White space and comments
//!   (`/* … */`) may stand between any two tokens and are dropped on read.
//! - **Reals (6.3.2).** A real has digits, a point, optional digits and an
//!   optional exponent. [`write`] writes the shortest decimal text that
//!   reads back as the same `f64`, in that form (`1.`, `0.25`, `1.E-7`):
//!   reals round-trip bit for bit. NaN and infinity cannot be written.
//! - **Strings (6.3.3).** Written as printable ASCII with `''` for an
//!   apostrophe and `\\` for a backslash; every other character goes
//!   through the `\X2\…\X0\` (UTF-16) or `\X4\…\X0\` (UTF-32) control
//!   directive. Read: those, plus `\X\hh` (ISO 8859-1), `\S\c` (the
//!   character 128 above `c` in the current ISO 8859 part; only part 1,
//!   the default, is decoded, a character of another part becomes U+FFFD)
//!   and `\Pc\`. Raw bytes above 127 are taken as UTF-8 when the string is
//!   valid UTF-8 (edition 3 allows that) and as ISO 8859-1 otherwise.
//! - **Header (clause 8)** and **data section (clause 9, 10)**: simple
//!   records, complex (multi-leaf) instances, lists, typed parameters
//!   (`LENGTH_MEASURE(1.5)`).
//!
//! Not covered: more than one data section, the `ANCHOR`, `REFERENCE` and
//! `SIGNATURE` sections of edition 3, and the `&SCOPE` construct of
//! edition 1. Each is an error that says so.
//!
//! Hostile input is an error with the byte offset, line and column where
//! it went wrong, never a panic: nesting, counts and sizes are capped by
//! [`ReadLimits`].
//!
//! Determinism: [`write`] emits instances in the order they are in the
//! file model, with the ids they carry; [`StepFile::add`] numbers them in
//! the order they are added.

mod model;
mod read;
mod write;

pub use model::{Entity, Header, Instance, Param, Record, StepFile};
pub use read::{ReadError, ReadErrorKind, ReadLimits, read, read_with};
pub use write::{WriteError, format_real, write};

#[cfg(test)]
mod tests;
