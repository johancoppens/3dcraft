//! `cargo xtask test` as a program: refused arguments end it with a failure
//! exit and the usage, before cargo is started. (That a failing test binary
//! fails the run is `ci.rs`'s unit test, through the same error path.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

#[test]
fn refused_arguments_exit_with_a_failure_and_the_usage() {
    for args in [
        &["test"][..],
        &["test", "--workspace"][..],
        &["test", "--test", "guide_examples"][..],
        &["test", "-p"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let said = String::from_utf8_lossy(&output.stderr);
        assert!(
            said.contains("usage: cargo xtask test -p <crate>"),
            "{args:?}: {said}"
        );
        assert!(!said.contains("$ cargo test"), "{args:?}: {said}");
    }
}
