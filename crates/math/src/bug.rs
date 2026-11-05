//! What every message about a bug in 3DCraft says: one wording, in
//! every crate, that tells the person what to do. The context (what went
//! wrong, where) follows it after a colon. Here because every crate that
//! can meet a bug depends on this one.
//!
//! The wording begins `internal error (a bug in 3DCraft`, which the fuzz
//! runner, `panic_hunt` and the window read as a bug found.

/// The words of a bug, with nothing after them.
pub const BUG: &str = crate::bug_text!();

/// A bug's message as a `&'static str`: [`BUG`], then `: ` and `context`
/// (a literal). `bug_text!()` alone is [`BUG`]. A message with values in
/// it is `format!("{BUG}: ...")`.
#[macro_export]
macro_rules! bug_text {
    () => {
        "internal error (a bug in 3DCraft, please report it, with session.report or Copy details)"
    };
    ($context:literal) => {
        concat!($crate::bug_text!(), ": ", $context)
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_bug_reads_as_one_with_its_context_after() {
        assert!(super::BUG.starts_with("internal error (a bug in 3DCraft, please report it"));
        assert_eq!(
            crate::bug_text!("lost a curve"),
            format!("{}: lost a curve", super::BUG)
        );
    }
}
