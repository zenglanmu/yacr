//! The application mode the command layer authorizes against.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Viewer,
    Work,
}
