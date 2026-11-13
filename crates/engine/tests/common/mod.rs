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

    use std::cell::Cell;

    /// Commands that only read the document: they leave its history, and
    /// so its revision, as it was.
    const READING: &[&str] = &[
        "document.inspect",
        "document.check",
        "document.measure",
        "document.edges",
        "sketch.inspect",
    ];

    /// Which document a session shows, as far as a test can tell: the
    /// session's address, the revision of its active document and what
    /// that document's last regeneration rebuilt, reused and compared.
    type State = (usize, Option<u64>, Option<(usize, usize)>, Option<usize>);

    fn state(session: &Session) -> State {
        (
            std::ptr::from_ref(session) as usize,
            session.document_revision(),
            session.last_regeneration(),
            session.last_regeneration_work(),
        )
    }

    thread_local! {
        /// The state the last check on this thread found to be a full
        /// regeneration.
        static CHECKED: Cell<Option<State>> = const { Cell::new(None) };
    }

    pub fn run(session: &mut Session, id: &str, params: Value) -> Value {
        let before = state(session);
        let result = super::plain::run(session, id, params.clone());
        let now = state(session);
        // A command that only reads, on the document the previous check
        // found sound and left at the same revision: the same document,
        // whose regeneration (kept per revision) was compared already, and
        // regeneration is deterministic. Checking again would repeat the
        // same comparison.
        if READING.contains(&id) && before == now && CHECKED.get() == Some(now) {
            return result;
        }
        // Built up edit by edit, the document is what regenerating it from
        // nothing gives (docs/testing.md: incremental = full regeneration).
        assert_ne!(
            session.regeneration_matches_a_full_one(),
            Some(false),
            "after {id} {params}"
        );
        CHECKED.set(Some(state(session)));
        result
    }

    /// Commands run as [`run`] runs them, the comparison with a
    /// regeneration from nothing after each one included, but done on
    /// threads of their own while the next commands run: a long script
    /// spends most of its time in those comparisons, each of which needs
    /// only the document as it was after its command and what the session
    /// held for it then. At most `at_once` comparisons are under way; the
    /// next command waits for the oldest rather than going on without its
    /// own. The verdicts are taken in the order of the commands, so the
    /// first command after which the two differ fails the test with
    /// [`run`]'s message, whatever the number of threads and whichever
    /// comparison ends first, and no later command is named.
    /// [`Overlapped::wait`] takes the last verdicts and holds the count:
    /// every command was compared, or was a reading command on a document
    /// already being compared (which [`run`] skips too), or had no document.
    pub struct Overlapped {
        at_once: usize,
        pending: std::collections::VecDeque<(String, std::thread::JoinHandle<bool>)>,
        /// The state the last comparison was started on.
        started: Option<State>,
        commands: usize,
        compared: usize,
        skipped: usize,
        without_document: usize,
        verdicts: usize,
        /// What a test that failed for another reason left unreported,
        /// besides stderr: for the helper's own tests.
        unreported: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Overlapped {
        pub fn new(at_once: usize) -> Self {
            Self {
                at_once: at_once.max(1),
                pending: std::collections::VecDeque::new(),
                started: None,
                commands: 0,
                compared: 0,
                skipped: 0,
                without_document: 0,
                verdicts: 0,
                unreported: std::sync::Arc::default(),
            }
        }

        pub fn run(&mut self, session: &mut Session, id: &str, params: Value) -> Value {
            let before = state(session);
            let result = super::plain::run(session, id, params.clone());
            let now = state(session);
            // As in `run`: a command that only reads, on the document whose
            // comparison was started last, would start the same comparison.
            if READING.contains(&id) && before == now && self.started == Some(now) {
                self.commands += 1;
                self.skipped += 1;
                return result;
            }
            match session.regeneration_to_compare() {
                Some(compare) => {
                    self.compare_after(format!("after {id} {params}"), move || compare.matches());
                }
                None => {
                    self.commands += 1;
                    self.without_document += 1;
                }
            }
            self.started = Some(state(session));
            result
        }

        /// A command's comparison, `after` naming the command as [`run`]'s
        /// message does: started on a thread of its own once fewer than
        /// `at_once` are under way. [`Self::run`] passes the session's; the
        /// helper's own tests pass comparisons of their making.
        pub fn compare_after(
            &mut self,
            after: String,
            comparison: impl FnOnce() -> bool + Send + 'static,
        ) {
            self.commands += 1;
            while self.pending.len() >= self.at_once {
                self.take_oldest();
            }
            self.pending
                .push_back((after, std::thread::spawn(comparison)));
            self.compared += 1;
        }

        /// The verdict of the oldest comparison under way. One that differs
        /// fails the test as [`run`] fails it, one that panicked fails it
        /// naming its command; either way the comparisons after it are
        /// waited for first and not reported, as [`run`] would not have
        /// reached them.
        fn take_oldest(&mut self) {
            let Some((after, comparison)) = self.pending.pop_front() else {
                return;
            };
            let verdict = comparison.join();
            self.verdicts += 1;
            if !matches!(verdict, Ok(true)) {
                for (_, later) in self.pending.drain(..) {
                    let _ = later.join();
                }
            }
            match verdict {
                Ok(matches) => assert_ne!(Some(matches), Some(false), "{after}"),
                Err(panicked) => {
                    let why = panicked
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panicked.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    panic!(
                        "the comparison with a regeneration from nothing {after} panicked: {why}"
                    );
                }
            }
        }

        /// Takes every verdict still to come, in order, and holds the count
        /// of commands to the comparisons made and the commands left out
        /// for a reason.
        pub fn finish(&mut self) {
            while !self.pending.is_empty() {
                self.take_oldest();
            }
            assert_eq!(
                self.verdicts, self.compared,
                "every comparison started gives its verdict"
            );
            assert_eq!(
                self.commands,
                self.compared + self.skipped + self.without_document,
                "every command is compared ({} compared, {} reading ones skipped, {} without a document)",
                self.compared,
                self.skipped,
                self.without_document
            );
        }

        /// [`Self::finish`], then leaves the session to [`run`] as if it had
        /// checked every command itself.
        pub fn wait(mut self, session: &Session) {
            self.finish();
            if self.started == Some(state(session)) {
                CHECKED.set(self.started);
            }
        }

        /// Where the comparisons a failing test left unreported are listed.
        pub fn unreported(&self) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
            std::sync::Arc::clone(&self.unreported)
        }
    }

    impl Drop for Overlapped {
        /// A test that ends without [`Overlapped::wait`] (it returned early)
        /// still has its verdicts taken here, and fails on one that
        /// differs. One that fails for another reason while comparisons are
        /// under way keeps its own failure, and the comparisons that failed
        /// are written out beside it, not dropped.
        fn drop(&mut self) {
            if std::thread::panicking() {
                for (after, comparison) in self.pending.drain(..) {
                    let said = match comparison.join() {
                        Ok(true) => continue,
                        Ok(false) => {
                            format!("the regeneration differed from one from nothing {after}")
                        }
                        Err(_) => format!(
                            "the comparison with a regeneration from nothing {after} panicked"
                        ),
                    };
                    eprintln!("also: {said}");
                    self.unreported
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(said);
                }
            } else {
                self.finish();
            }
        }
    }

    through_run!();
}
