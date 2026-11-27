//! The typed `SECTIONPLANE` payload.
//!
//! The command writes one document section plane (`tables/sections.json`). A
//! bare selector toggles the stored plane; `SET` replaces it; `CLEAR` removes
//! it; `LIST` reports it without writing.

use cad_domain::Point3;

/// One `SECTIONPLANE` option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SectionCommand {
    /// `SECTIONPLANE SET <ox> <oy> <oz> <nx> <ny> <nz>`.
    Set { origin: Point3, normal: Point3 },
    /// `SECTIONPLANE ON`.
    On,
    /// `SECTIONPLANE OFF`.
    Off,
    /// Bare `SECTIONPLANE` / `SECTIONPLANE TOGGLE`.
    Toggle,
    /// `SECTIONPLANE CLEAR`.
    Clear,
    /// `SECTIONPLANE LIST` (also bare with no stored plane? no: bare toggles).
    List,
}
