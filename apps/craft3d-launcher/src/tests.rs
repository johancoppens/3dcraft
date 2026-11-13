use super::*;

/// Held by the tests that make error files, so the one that counts them
/// sees only its own.
static ERROR_FILES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An exit status with `code`, as the platform makes one.
fn status(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(u32::from_ne_bytes(code.to_ne_bytes()))
    }
}

#[test]
fn the_window_is_the_program_beside_the_launcher() {
    let folder = std::env::temp_dir().join("Program Files").join("3DCraft");
    let launcher = folder.join(format!("3DCraft{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(
        window_beside(&launcher),
        Some(folder.join(format!("craft3d{}", std::env::consts::EXE_SUFFIX)))
    );
    // A path without a folder has none to look in.
    assert_eq!(window_beside(Path::new("/")), None);
}

/// A folder whose name is not UTF-8 (possible on Linux; on Windows a name
/// with an unpaired surrogate) is kept as it is.
#[cfg(unix)]
#[test]
fn a_folder_that_is_not_utf8_is_kept() {
    use std::os::unix::ffi::OsStrExt;
    let launcher = Path::new(OsStr::from_bytes(b"/opt/3d\xffcraft/3DCraft"));
    let window = window_beside(launcher).unwrap();
    assert_eq!(window.as_os_str().as_bytes(), b"/opt/3d\xffcraft/craft3d");
}

/// The arguments go on as they came, one by one: a double-clicked file's
/// path with spaces, quotes, backslashes, an empty argument (std quotes
/// each for the program's command line on Windows).
#[test]
fn the_arguments_are_passed_on_unchanged() {
    let args: Vec<OsString> = [
        r"C:\Users\An Example\My Designs\bracket v2.3dcraft",
        "say \"hello\"",
        r"trailing\",
        "",
        "--control",
        "57312",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let command = window_command(Path::new("craft3d"), args.clone(), None);
    let passed: Vec<&OsStr> = command.get_args().collect();
    assert_eq!(
        passed,
        args.iter().map(OsString::as_os_str).collect::<Vec<_>>()
    );
    assert_eq!(command.get_program(), OsStr::new("craft3d"));
}

#[test]
fn the_last_message_is_the_windows_last_one() {
    // One line.
    assert_eq!(
        last_message("craft3d: the window could not be opened: no display\n"),
        "craft3d: the window could not be opened: no display"
    );
    // A message over several lines, after an earlier one: from its first
    // line to the end, with Windows' line ends.
    let said = "craft3d: control channel on 127.0.0.1:57312\r\n\
                craft3d: the window could not be opened: no graphics adapter can show it (none).\r\n\
                3DCraft renders the model on the CPU.\r\n  \
                Windows: it comes with the system (WARP)\r\n\
                Everything but the window works without any of this.\r\n\r\n";
    assert_eq!(
        last_message(said),
        "craft3d: the window could not be opened: no graphics adapter can show it (none).\n\
         3DCraft renders the model on the CPU.\n  \
         Windows: it comes with the system (WARP)\n\
         Everything but the window works without any of this."
    );
    // Without a message of the window's own (a crash): its last five lines
    // that say something, the last line among them.
    let crash = "a\nb\nthread 'main' panicked at src/x.rs:1:1:\nboom\n\nnote: run with \
                 `RUST_BACKTRACE=1`\nlast\n";
    assert_eq!(
        last_message(crash),
        "b\nthread 'main' panicked at src/x.rs:1:1:\nboom\n\nnote: run with `RUST_BACKTRACE=1`\nlast"
    );
    // Nothing said.
    assert_eq!(last_message(""), "");
    assert_eq!(last_message("\n \r\n\t\n"), "");
    // No control characters; a tab stays.
    assert_eq!(
        last_message("craft3d: a\u{1b}[31mb\tc\u{7}\n"),
        "craft3d: a[31mb\tc"
    );
    // Capped in lines and in characters.
    let long: String = (0..100)
        .map(|i| format!("craft3d: {i}\n  more\n"))
        .collect();
    assert_eq!(last_message(&long), "craft3d: 99\n  more");
    // The caps keep the end, where the error is: the last 40 lines, the
    // last 4000 characters.
    let tall = format!("craft3d: start\n{}the end\n", "line\n".repeat(100));
    let message = last_message(&tall);
    assert_eq!(message.lines().count(), 40);
    assert!(message.ends_with("line\nthe end"), "{message}");
    assert!(!message.contains("start"), "{message}");
    let wide = format!("craft3d: {}y\n", "x".repeat(10_000));
    let message = last_message(&wide);
    assert_eq!(message.chars().count(), 4001);
    assert!(
        message.starts_with('…') && message.ends_with("xy"),
        "{message}"
    );
}

#[test]
fn exit_codes_read_as_windows_people_read_them() {
    assert_eq!(exit_code_text(Some(1)), "1");
    assert_eq!(exit_code_text(Some(2)), "2");
    // STATUS_DLL_NOT_FOUND, as Windows reports a missing DLL.
    assert_eq!(exit_code_text(Some(-1_073_741_515)), "0xC0000135");
    assert_eq!(exit_code_text(None), "none (ended by a signal)");
}

/// Something is said only for a window that ended with an error within
/// [`START_UP`], always with its exit code: that it could not start, with
/// its last message as the reason; or, once it had said it started (the
/// renderer's line), that it ended unexpectedly, never quoting the news of
/// its start as the reason.
#[test]
fn what_is_said_when_the_window_ends_with_an_error() {
    let quick = Duration::from_secs(2);
    let said = "craft3d: the window could not be opened: no display\n";
    assert_eq!(start_up_failure(status(0), quick, said), None);
    assert_eq!(
        start_up_failure(status(1), quick, said).as_deref(),
        Some(
            "3DCraft could not start (exit code 1).\n\n\
             craft3d: the window could not be opened: no display"
        )
    );
    // After the start-up: killed, or a crash after work, whose recovery
    // files the next start offers.
    assert_eq!(start_up_failure(status(1), START_UP, said), None);
    assert_eq!(
        start_up_failure(status(2), quick, "").as_deref(),
        Some("3DCraft could not start (exit code 2), and the window gave no reason.")
    );
    // The control channel's line is news, not a reason.
    assert_eq!(
        start_up_failure(
            status(3),
            quick,
            "craft3d: control channel on 127.0.0.1:5 (token in t)\n"
        )
        .as_deref(),
        Some("3DCraft could not start (exit code 3), and the window gave no reason.")
    );
    // Started (the renderer's line), then ended without a word: a driver's
    // crash, End task.
    let started = "craft3d: control channel on 127.0.0.1:5 (token in t)\n\
                   craft3d: viewport renderer: wgpu on WARP (dx12)\n";
    assert_eq!(
        start_up_failure(status(1), quick, started).as_deref(),
        Some("3DCraft ended unexpectedly (exit code 1).")
    );
    // Started, then a crash that said why: its lines are the reason.
    let crashed = format!(
        "{started}thread 'main' panicked at src/x.rs:1:1:\nboom\nnote: run with `RUST_BACKTRACE=1`\n"
    );
    assert_eq!(
        start_up_failure(status(101), quick, &crashed).as_deref(),
        Some(
            "3DCraft ended unexpectedly (exit code 101).\n\n\
             thread 'main' panicked at src/x.rs:1:1:\nboom\nnote: run with `RUST_BACKTRACE=1`"
        )
    );
    // Started, then a message of the window's own.
    let failed = format!("{started}craft3d: the device was lost\n");
    assert_eq!(
        start_up_failure(status(1), quick, &failed).as_deref(),
        Some("3DCraft ended unexpectedly (exit code 1).\n\ncraft3d: the device was lost")
    );
}

/// The window's words the launcher reads (its messages, its start, its
/// news) are still the window's (`apps/craft3d`).
#[test]
fn the_window_still_says_what_the_launcher_reads() {
    let window = Path::new(env!("CARGO_MANIFEST_DIR")).join("../craft3d/src");
    let main = std::fs::read_to_string(window.join("main.rs")).unwrap();
    assert!(main.contains("&format!(\"craft3d: {error}\")"));
    let app = std::fs::read_to_string(window.join("app.rs")).unwrap();
    for said in [
        "\"viewport renderer: {}\"",
        "\"control channel on {address} (token in {})\"",
        "unsaved document(s) of an earlier session can be recovered",
    ] {
        assert!(app.contains(said), "{said}");
    }
    assert!(STARTED.starts_with(MESSAGE_START) && NEWS.contains(&STARTED));
}

/// Each start has an error file of its own, made new.
#[test]
fn each_start_has_an_error_file_of_its_own() {
    let _held = ERROR_FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (first, _) = error_file().unwrap();
    let (second, _) = error_file().unwrap();
    assert_ne!(first, second);
    let name = first.file_name().unwrap().to_str().unwrap().to_string();
    assert!(
        name.starts_with(&format!("3dcraft-launcher-{}-", std::process::id())),
        "{name}"
    );
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
}

/// A folder of the test's own in the temporary folder, empty.
fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "craft3d-launcher-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// With the switch set to a file, what would be in the box is one line
/// appended to it.
#[test]
fn the_switch_writes_a_line_instead_of_a_box() {
    let folder = folder("switch");
    let log = folder.join("launcher.log");
    report(
        "3DCraft could not start.\n\ncraft3d: no display\n  more help",
        Some(log.as_os_str()),
    );
    report("second", Some(log.as_os_str()));
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "craft3d-launcher: 3DCraft could not start. | craft3d: no display |   more help\n\
         craft3d-launcher: second\n"
    );
    // A switch that names no file falls back to standard error: no box, no
    // panic.
    report("third", Some(folder.join("no/such/folder/log").as_os_str()));
    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(LOG_SWITCH, "CRAFT3D_LAUNCHER_LOG");
}

/// A missing window is said, naming where it was looked for.
#[test]
fn a_missing_window_is_said() {
    let folder = folder("missing");
    let launcher = folder.join("3DCraft");
    let ended = launch(&launcher, Vec::new());
    assert_eq!(ended.code, None);
    let error = ended.say.unwrap();
    assert!(error.starts_with("3DCraft could not start: "), "{error}");
    assert!(error.contains("craft3d"), "{error}");
    assert!(error.contains("was not found beside"), "{error}");
    std::fs::remove_dir_all(&folder).unwrap();
}

/// A stand-in window: a shell script named as the window, beside a
/// launcher path in a folder of its own.
#[cfg(unix)]
fn stand_in(name: &str, script: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let folder = folder(name);
    let window = folder.join(WINDOW);
    std::fs::write(&window, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&window, std::fs::Permissions::from_mode(0o755)).unwrap();
    (folder.join("3DCraft"), folder)
}

/// The launcher with stand-in windows, one after the other (so the error
/// files of this process can be counted): the window beside it gets the
/// arguments as they came, and one that ends well leaves nothing to say;
/// one that could not start is said with its last message and its code, one
/// that had started and then ended with its code; one that writes
/// far more than a pipe holds is never held up (its errors go to a file,
/// not to a pipe the launcher must drain). No error file is left behind.
#[cfg(unix)]
#[test]
fn the_launcher_starts_the_window_beside_it() {
    let _held = ERROR_FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = files_of_ours();

    let (launcher, folder) = stand_in(
        "arguments",
        "printf '%s\\0' \"$@\" > \"$(dirname \"$0\")/args\"",
    );
    let args: Vec<OsString> = ["my designs/bracket \"v2\".3dcraft", "", "--x=a b"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(
        launch(&launcher, args.clone()),
        Ended {
            code: Some(0),
            say: None
        }
    );
    let written = std::fs::read(folder.join("args")).unwrap();
    let got: Vec<&[u8]> = written.split(|b| *b == 0).collect();
    let want: Vec<&[u8]> = args
        .iter()
        .map(|a| a.to_str().unwrap().as_bytes())
        .chain([&b""[..]])
        .collect();
    assert_eq!(got, want);
    std::fs::remove_dir_all(&folder).unwrap();

    let (launcher, folder) = stand_in(
        "fails",
        "echo 'craft3d: control channel on 127.0.0.1:1' >&2\n\
         echo 'craft3d: the window could not be opened: no graphics adapter' >&2\n\
         echo '  Windows: it comes with the system (WARP)' >&2\n\
         exit 1",
    );
    assert_eq!(
        launch(&launcher, Vec::new()),
        Ended {
            code: Some(1),
            say: Some(
                "3DCraft could not start (exit code 1).\n\n\
                 craft3d: the window could not be opened: no graphics adapter\n  \
                 Windows: it comes with the system (WARP)"
                    .to_string()
            )
        }
    );
    std::fs::remove_dir_all(&folder).unwrap();

    // A window that had started and then ended without a word: its code,
    // passed on.
    let (launcher, folder) = stand_in(
        "ends",
        "echo 'craft3d: viewport renderer: wgpu (vulkan)' >&2\nexit 3",
    );
    assert_eq!(
        launch(&launcher, Vec::new()),
        Ended {
            code: Some(3),
            say: Some("3DCraft ended unexpectedly (exit code 3).".to_string())
        }
    );
    std::fs::remove_dir_all(&folder).unwrap();

    let line = format!("echo 'craft3d: {}' >&2", ".".repeat(500));
    let (launcher, folder) = stand_in(
        "much",
        &format!("i=0\nwhile [ $i -lt 2000 ]; do\n  {line}\n  i=$((i+1))\ndone\nexit 0"),
    );
    let started = Instant::now();
    assert_eq!(launch(&launcher, Vec::new()).code, Some(0));
    assert!(started.elapsed() < Duration::from_secs(60));
    std::fs::remove_dir_all(&folder).unwrap();

    assert_eq!(files_of_ours(), before);
}

/// The launcher's error files of this process in the temporary folder.
#[cfg(unix)]
fn files_of_ours() -> Vec<String> {
    let prefix = format!("3dcraft-launcher-{}-", std::process::id());
    let mut names: Vec<String> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    names.sort();
    names
}
