//! `3DCraft.exe` in the Windows packages: starts the window, `craft3d.exe`
//! beside it, without a console window (the library says how). A program
//! of the Windows GUI subsystem, so it opens no console of its own; on
//! other systems an ordinary program that does the same.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let log = std::env::var_os(craft3d_launcher::LOG_SWITCH);
    let launcher = match std::env::current_exe() {
        Ok(launcher) => launcher,
        Err(error) => {
            craft3d_launcher::report(
                &format!("3DCraft could not start: the launcher cannot find itself: {error}"),
                log.as_deref(),
            );
            return ExitCode::FAILURE;
        }
    };
    let args = std::env::args_os().skip(1).collect();
    let ended = craft3d_launcher::launch(&launcher, args);
    if let Some(say) = &ended.say {
        craft3d_launcher::report(say, log.as_deref());
    }
    match ended.code {
        // The window's own exit code, whole (on Windows an NTSTATUS does not
        // fit the u8 of `ExitCode`).
        Some(code) => std::process::exit(code),
        None => ExitCode::FAILURE,
    }
}
