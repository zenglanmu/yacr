//! The session mode, split out of `command_id.rs` to keep the enum file inside
//! the code-size limit. Re-exported, so `cad_app::AppMode` is unchanged.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Viewer,
    Work,
}
