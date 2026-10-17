//! Shared by the engine's integration tests: a session, commands that must
//! succeed, fail or be refused, and what the document says about a body or
//! a feature. A test file takes what it uses with `mod common;` and
//! `use common::checked::{..}` (every command it runs is also held to
//! "regenerating the document from nothing gives the same", docs/testing.md:
//! incremental = full regeneration) or `use common::plain::{..}` (commands
//! only).

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    unused_imports,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::Value;
use std::path::PathBuf;

/// A session that may neither read nor write files.
pub fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

/// A session that reads and writes in `dir` only.
pub fn session_in(dir: &std::path::Path) -> Session {
    Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    })
}

/// An empty directory `<prefix>-<name>` in the test target's scratch place.
pub fn workdir(prefix: &str, name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{prefix}-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A command that must be refused: the error, which is not an internal one.
pub fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should be refused, returned {value}"),
        Err(e) => {
            assert!(!e.message().contains("internal error"), "{id}: {e}");
            e.to_string()
        }
    }
}

/// A command that must fail: the error, which is not an internal one.
pub fn error(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => {
            assert!(!e.message().contains("internal error"), "{id}: {e}");
            e.to_string()
        }
    }
}

/// A command that must be refused, without a look at the error.
pub fn refused_with(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(result) => panic!("{id} {params} should have been refused: {result}"),
        Err(error) => error.to_string(),
    }
}

pub fn str_of(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("not a string: {value}"))
        .to_string()
}

#[track_caller]
pub fn assert_close(actual: &Value, expected: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

pub fn names_of(body: &Value) -> Vec<String> {
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

/// The helpers that run commands, the same in [`plain`] and [`checked`]
/// but for the `run` they go through.
macro_rules! through_run {
    () => {
        pub use super::{
            assert_close, error, names_of, refused, refused_with, session, session_in, str_of,
            workdir,
        };

        /// A command that adds or edits a feature, which has to regenerate.
        pub fn built(session: &mut Session, id: &str, params: Value) -> Value {
            let result = run(session, id, params.clone());
            assert_eq!(result["status"], "ok", "{id} {params}: {result}");
            result
        }

        /// A command that is accepted but whose feature fails: the reason.
        pub fn failed(session: &mut Session, id: &str, params: Value) -> String {
            let result = run(session, id, params.clone());
            assert_eq!(result["status"], "failed", "{id} {params}: {result}");
            result["error"].as_str().unwrap().to_string()
        }

        /// A new document in millimetres whose tolerance fits parts of
        /// size `k`.
        pub fn new_document(session: &mut Session, k: f64) {
            run(
                session,
                "file.new",
                json!({"units": "mm", "tolerance": 1e-6 * k}),
            );
        }

        pub fn inspect(session: &mut Session) -> Value {
            run(session, "document.inspect", json!({}))
        }

        pub fn body(session: &mut Session, name: &str) -> Value {
            let document = inspect(session);
            document["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["name"] == name)
                .unwrap_or_else(|| panic!("no body {name} in {document}"))
                .clone()
        }

        pub fn feature(session: &mut Session, name: &str) -> Value {
            let document = inspect(session);
            document["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["name"] == name)
                .unwrap_or_else(|| panic!("no feature {name} in {document}"))
                .clone()
        }

        /// [`feature`] whose failure names the feature only.
        pub fn feature_brief(session: &mut Session, name: &str) -> Value {
            inspect(session)["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["name"] == name)
                .unwrap_or_else(|| panic!("no feature {name}"))
                .clone()
        }

        /// The reason the feature `name` failed.
        pub fn failure(session: &mut Session, name: &str) -> String {
            let state = feature_brief(session, name);
            assert_eq!(state["status"], "failed", "{state}");
            state["error"].as_str().unwrap().to_string()
        }

        /// The deep check passes.
        pub fn deep(session: &mut Session) {
            let check = run(session, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{check}");
        }

        /// The body is valid, also to the mesh-level check, and has this
        /// volume and area (given for scale 1).
        #[track_caller]
        pub fn assert_solid(
            session: &mut Session,
            name: &str,
            volume: f64,
            area: Option<f64>,
            k: f64,
        ) -> Value {
            let found = body(session, name);
            assert_eq!(found["valid"], true, "{found}");
            assert_close(&found["volume"], volume * k * k * k, "volume");
            if let Some(area) = area {
                assert_close(&found["area"], area * k * k, "area");
            }
            let check = run(session, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{check}");
            assert_eq!(check["problems"], json!([]), "{check}");
            found
        }
    };
}

/// Commands run as they are.
pub mod plain {
    use craft3d_engine::Session;
    use serde_json::{Value, json};

    pub fn run(session: &mut Session, id: &str, params: Value) -> Value {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
    }

    through_run!();
}

/// Commands run and the document then held to a regeneration from
/// nothing.
pub mod checked {
    use craft3d_engine::Session;
    use serde_json::{Value, json};

    pub fn run(session: &mut Session, id: &str, params: Value) -> Value {
        let result = super::plain::run(session, id, params.clone());
        // Built up edit by edit, the document is what regenerating it from
        // nothing gives (docs/testing.md: incremental = full regeneration).
        assert_ne!(
            session.regeneration_matches_a_full_one(),
            Some(false),
            "after {id} {params}"
        );
        result
    }

    through_run!();
}
