//! Hand-written argument parsing: flags with values, positionals, and
//! errors that name what was wrong.
//!
//! The conventions: `--flag VALUE` or `--flag=VALUE`; `--` ends the flags,
//! so a file name may start with a dash after it; `-` alone is a
//! positional. A flag's value is always the next argument, whatever it
//! looks like, so JSON and negative numbers need no escaping.

use crate::CliError;
use std::collections::VecDeque;
use std::ffi::OsString;

/// One argument, classified.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Arg {
    /// `--name`, with the value of `--name=value` when it was written so.
    Flag {
        name: String,
        inline: Option<String>,
    },
    Positional(String),
}

pub(crate) struct Args {
    rest: VecDeque<String>,
    flags_ended: bool,
}

impl Args {
    /// Takes the process arguments after the program name. An argument
    /// that is not Unicode is a usage error (`std::env::args` would panic).
    pub fn from_os(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let mut rest = VecDeque::new();
        for arg in args {
            match arg.into_string() {
                Ok(arg) => rest.push_back(arg),
                Err(bad) => {
                    return Err(CliError::Usage(format!(
                        "argument `{}` is not valid Unicode",
                        bad.to_string_lossy()
                    )));
                }
            }
        }
        Ok(Self {
            rest,
            flags_ended: false,
        })
    }

    /// The next argument, or `None` at the end.
    pub fn next(&mut self) -> Option<Arg> {
        let arg = self.rest.pop_front()?;
        if self.flags_ended {
            return Some(Arg::Positional(arg));
        }
        if arg == "--" {
            self.flags_ended = true;
            return self.next();
        }
        if arg.starts_with('-') && arg.len() > 1 {
            return Some(match arg.split_once('=') {
                Some((name, value)) => Arg::Flag {
                    name: name.to_string(),
                    inline: Some(value.to_string()),
                },
                None => Arg::Flag {
                    name: arg,
                    inline: None,
                },
            });
        }
        Some(Arg::Positional(arg))
    }

    /// The value of the flag just read: its inline value, or the next
    /// argument.
    pub fn value(&mut self, flag: &str, inline: Option<String>) -> Result<String, CliError> {
        inline
            .or_else(|| self.rest.pop_front())
            .ok_or_else(|| CliError::Usage(format!("`{flag}` needs a value")))
    }
}

/// A flag that takes no value was given one (`--help=yes`).
impl Args {
    /// The value of a flag whose value may be left out: the inline one, or
    /// the next argument when it is not a flag.
    pub fn optional_value(&mut self, inline: Option<String>) -> Option<String> {
        if inline.is_some() {
            return inline;
        }
        match self.rest.front() {
            Some(next) if !next.starts_with('-') && !self.flags_ended => self.rest.pop_front(),
            _ => None,
        }
    }
}

pub(crate) fn no_value(flag: &str, inline: Option<&String>) -> Result<(), CliError> {
    match inline {
        None => Ok(()),
        Some(_) => Err(CliError::Usage(format!("`{flag}` takes no value"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        Args::from_os(list.iter().map(OsString::from)).unwrap()
    }

    fn flag(name: &str, inline: Option<&str>) -> Option<Arg> {
        Some(Arg::Flag {
            name: name.to_string(),
            inline: inline.map(str::to_string),
        })
    }

    fn positional(text: &str) -> Option<Arg> {
        Some(Arg::Positional(text.to_string()))
    }

    #[test]
    fn classifies_flags_and_positionals() {
        let mut a = args(&[
            "part.3dcraft",
            "--out",
            "x.png",
            "--size=4x3",
            "-h",
            "-",
            "--",
            "--odd",
            "--",
        ]);
        assert_eq!(a.next(), positional("part.3dcraft"));
        assert_eq!(a.next(), flag("--out", None));
        assert_eq!(a.value("--out", None).unwrap(), "x.png");
        assert_eq!(a.next(), flag("--size", Some("4x3")));
        assert_eq!(a.next(), flag("-h", None));
        assert_eq!(a.next(), positional("-"));
        // After `--` everything is a positional, a second `--` included.
        assert_eq!(a.next(), positional("--odd"));
        assert_eq!(a.next(), positional("--"));
        assert_eq!(a.next(), None);
    }

    #[test]
    fn a_value_is_taken_as_it_is() {
        let mut a = args(&[
            "--params",
            "{\"a\": \"b=c\"}",
            "--tolerance",
            "-1",
            "--new={\"units\":\"mm\"}",
            "--out",
        ]);
        assert_eq!(a.next(), flag("--params", None));
        assert_eq!(a.value("--params", None).unwrap(), "{\"a\": \"b=c\"}");
        assert_eq!(a.next(), flag("--tolerance", None));
        assert_eq!(a.value("--tolerance", None).unwrap(), "-1");
        // Only the first `=` splits.
        assert_eq!(a.next(), flag("--new", Some("{\"units\":\"mm\"}")));
        assert_eq!(a.next(), flag("--out", None));
        let error = a.value("--out", None).unwrap_err();
        assert!(matches!(error, CliError::Usage(ref m) if m.contains("`--out` needs a value")));
    }

    #[cfg(unix)]
    #[test]
    fn an_argument_that_is_not_unicode_is_a_usage_error() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![b'a', 0xff, b'b']);
        let error = Args::from_os([bad].into_iter()).err().unwrap();
        assert!(matches!(error, CliError::Usage(ref m) if m.contains("not valid Unicode")));
    }
}
