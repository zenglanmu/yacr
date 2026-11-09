//! `yacr-windows` desktop binary.
//!
//! Delegates to [`app_windows::entry::run`]. On Windows it runs the real
//! Tauri/WebView2/wgpu host; on every other target it returns an explicit NOT
//! IMPLEMENTED error (no GUI dependencies are active there).
//!
//! The binary name is deliberately `yacr-windows`, distinct from the Linux
//! product binary `yacr` (`apps/app-linux`): both packages compile for the same
//! host target during workspace checks, so a shared name would make the two
//! hosts compete for (or overwrite) one output artifact path.
//!
//! # Console subsystem
//!
//! A packaged desktop app must not open a console window, so a **release** build
//! links as a GUI-subsystem binary (`windows_subsystem = "windows"`). The
//! `[probe]` lines are the Windows evidence stream, therefore a release build
//! started *from* a terminal re-attaches to that parent console and reopens its
//! standard handles; a double-click has no parent console and stays silent.
//! Debug builds keep the console subsystem, so a `cargo run` in a terminal
//! behaves exactly as before.

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    attach_parent_console();
    if let Err(error) = app_windows::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// Re-open the parent terminal's console for a GUI-subsystem release binary.
///
/// `AttachConsole` alone does not give this process usable `stdout`/`stderr`
/// handles, so the console output device is opened explicitly and installed as
/// both standard handles **before** the first diagnostic is printed (Rust caches
/// the handle on first use). Failure is not an error: a double-click simply has
/// no console to talk to, which is the product behaviour.
#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn attach_parent_console() {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };

    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return;
    }
    let opened = unsafe {
        CreateFileW(
            w!("CONOUT$"),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    let Ok(handle) = opened else {
        return;
    };
    unsafe {
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
        let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
    }
}
