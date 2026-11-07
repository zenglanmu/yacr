//! Session-only display tokens from the 2026-11-07 extras batch: `VIEWRES` and
//! `VSCURRENT`.
//!
//! Both are **session view state only**: no database write, revision bump,
//! `cad-history` entry or `ChangeSet`. `ViewExtras` is stored on
//! [`crate::SessionState`] so a host can read the value back; this lane wires no
//! renderer change, so the values are not pixel- or GPU-verified.

use cad_domain::{CadError, CadResult};

/// The accepted `VIEWRES` range (AutoCAD's own 1..=1000 percentage).
pub const VIEWRES_MIN: u32 = 1;
/// The accepted `VIEWRES` maximum.
pub const VIEWRES_MAX: u32 = 1000;
/// This build's declared default: AutoCAD's modern default of 1000.
pub const VIEWRES_DEFAULT: u32 = 1000;

/// The visual styles `VSCURRENT` can store.
///
/// Only the 2D wireframe token exists. AutoCAD's `Conceptual`, `Realistic`,
/// `Shaded`, `Shaded with edges`, `Hidden`, `Sketchy` and the 3D wireframe forms
/// are **NOT IMPLEMENTED** and are refused by name at the command layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualStyleToken {
    /// 2D wireframe.
    Wireframe,
}

impl VisualStyleToken {
    /// The stable token spelling a host reads.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wireframe => "WIREFRAME",
        }
    }
}

/// Session display tokens that are not drawing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewExtras {
    /// `VIEWRES` circle-tessellation percentage (`1..=1000`).
    pub viewres_percent: u32,
    /// `VSCURRENT` visual-style token.
    pub visual_style: VisualStyleToken,
}

impl Default for ViewExtras {
    fn default() -> Self {
        Self {
            viewres_percent: VIEWRES_DEFAULT,
            visual_style: VisualStyleToken::Wireframe,
        }
    }
}

impl ViewExtras {
    /// Store one `VIEWRES` percentage, refusing a value outside `1..=1000`
    /// instead of clamping it.
    pub fn set_viewres(&mut self, percent: u32) -> CadResult<()> {
        if !(VIEWRES_MIN..=VIEWRES_MAX).contains(&percent) {
            return Err(CadError::InvalidInput(format!(
                "viewres.out_of_range: VIEWRES needs a circle-tessellation percentage in \
                 {VIEWRES_MIN}..={VIEWRES_MAX}, got {percent}; nothing was changed"
            )));
        }
        self.viewres_percent = percent;
        Ok(())
    }

    /// Store the 2D wireframe token. Any other style is a named refusal.
    pub fn set_visual_style(&mut self, style: &str) -> CadResult<()> {
        if !style.eq_ignore_ascii_case("WIREFRAME") {
            return Err(CadError::Unsupported(format!(
                "vscurrent.style_not_implemented: VSCURRENT only supports WIREFRAME in this \
                 build; `{style}` is NOT IMPLEMENTED and the visual style was not changed"
            )));
        }
        self.visual_style = VisualStyleToken::Wireframe;
        Ok(())
    }
}
