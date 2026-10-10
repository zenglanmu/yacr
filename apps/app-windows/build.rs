//! Embed the application icon into `yacr.exe`.
//!
//! Only Windows targets carry a PE resource section; on other hosts (the crate
//! is still compiled for `--lib`/`--all-targets` checks) the build script is a
//! no-op. `winresource` drives `rc.exe` on MSVC and `windres` on GNU, so the
//! icon is embedded both in the Windows CI release and in a local MinGW cross
//! build. A missing resource compiler fails the build loudly rather than
//! silently shipping an icon-less executable.
fn main() {
    // The icon is the shared single source of truth; rebuild if it changes.
    println!("cargo:rerun-if-changed=../../assets/yacr-icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/yacr-icon.ico");
    resource.set("ProductName", "yacr");
    resource.set("FileDescription", "yacr CAD");
    if let Err(error) = resource.compile() {
        panic!("failed to compile the Windows icon resource: {error}");
    }
}
