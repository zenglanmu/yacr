//! Desktop CAD font loading: the sibling `fonts/` package plus the system
//! default.
//!
//! This is a thin alias of the shared native loader in
//! [`cad_platform::fonts::local`], which both the desktop host and the CLI use:
//!
//! 1. the `fonts/` directory next to the executable (or `--fonts-dir`), read
//!    through the shared catalog-driven loader;
//! 2. explicit `--font NAME=PATH` registrations;
//! 3. a best-effort system default outline face (fontconfig `fc-match`), which
//!    becomes the first fallback so a drawing font that is missing shapes with
//!    the platform default instead of being dropped.
//!
//! Because the font package is committed to the repository and copied next to
//! the binary at build time, a packaged client needs no network for the default
//! face.

pub use cad_platform::fonts::local::{
    cached_system_default_candidates, load_engine as load_desktop_fonts, resolve_font_dir,
};
