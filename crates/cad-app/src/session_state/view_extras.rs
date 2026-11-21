//! Session-only display tokens from the 2026-11-07 extras batch: `VIEWRES`,
//! `VSCURRENT` and `IBL`.
//!
//! All three are **session view state only**: no database write, revision bump,
//! `cad-history` entry or `ChangeSet`.
//!
//! * `VIEWRES` is **consumed** by the display build: the percentage becomes a
//!   ceiling on chord segments per curve
//!   ([`cad_representation::viewres_segment_ceiling`],
//!   [`cad_representation::RepresentationContext::curve_segments`]). The ceiling
//!   is deliberately partial — analytic conics (circles/arcs/ellipses) have no
//!   chords — so the command reports exactly that.
//! * `VSCURRENT` stores the 3D **body** display style. It is not a dead token:
//!   `WIREFRAME` makes every `Body3d` draw its edges only, `SHADED` keeps the
//!   filled mesh plus its stored-edge overlay
//!   ([`cad_representation::RepresentationContext::body_wireframe`]). The token
//!   is session-only and affects 3D bodies only; other geometry keeps its
//!   display. No pixel/GPU effect is claimed or verified for either token.

use cad_domain::{CadError, CadResult};

/// The accepted `VIEWRES` range (AutoCAD's own 1..=1000 percentage).
pub const VIEWRES_MIN: u32 = 1;
/// The accepted `VIEWRES` maximum.
pub const VIEWRES_MAX: u32 = 1000;
/// This build's declared default: AutoCAD's modern default of 1000.
pub const VIEWRES_DEFAULT: u32 = 1000;

/// The visual styles `VSCURRENT` can store.
///
/// Two tokens exist because this build implements two 3D-body display modes:
///
/// * [`Self::Shaded`] — the filled mesh plus the body's stored-edge overlay.
///   This is the **default**, which preserves the display this build had before
///   `VSCURRENT` was wired to the representation. AutoCAD's own initial value for
///   a new drawing is 2D wireframe; this build deliberately diverges so the
///   existing filled body display does not change behind a user's back.
/// * [`Self::Wireframe`] — the 3D-wireframe form: body edges only (stored edges,
///   or triangle-derived segments when a primitive has no stored boundary
///   edges, so a sphere/torus does not vanish).
///
/// AutoCAD's `Conceptual`, `Realistic`, `Hidden`, `Sketchy`, `X-ray`, the flat
/// forms and every other style are **NOT IMPLEMENTED** and are refused by name
/// at the command layer — a refusal, never a silent mapping onto either token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualStyleToken {
    /// 3D body wireframe: edges only.
    Wireframe,
    /// 3D body shaded display: filled mesh plus stored edges (the default).
    Shaded,
}

impl VisualStyleToken {
    /// The stable token spelling a host reads.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wireframe => "WIREFRAME",
            Self::Shaded => "SHADED",
        }
    }
}

/// The `IBL` PBR environment-lighting switch.
///
/// This is the **value** the procedural image-based-lighting lane reads at
/// render time; this crate only stores and reports it. Session-only: no database
/// write, revision bump, `cad-history` entry, `ChangeSet` or persisted
/// preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PbrEnvironment {
    /// Legacy single-direction lighting (`LEGACY`/`OFF`/`0`).
    Legacy,
    /// Procedural image-based lighting (`IBL`/`ON`/`1`/`IMAGEBASED`), the
    /// default so the pre-toggle scene is unchanged.
    #[default]
    ImageBased,
}

impl PbrEnvironment {
    /// The stable token spelling a host reads.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Legacy => "LEGACY",
            Self::ImageBased => "IBL",
        }
    }

    /// Whether procedural image-based lighting is active.
    pub const fn is_image_based(self) -> bool {
        matches!(self, Self::ImageBased)
    }
}

/// Session display tokens that are not drawing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewExtras {
    /// `VIEWRES` circle-tessellation percentage (`1..=1000`).
    ///
    /// Consumed by the display build as a chord-segment ceiling per curve (see
    /// [`cad_representation::viewres_segment_ceiling`]); the default is
    /// AutoCAD's modern 1000.
    pub viewres_percent: u32,
    /// `VSCURRENT` visual-style token.
    pub visual_style: VisualStyleToken,
    /// `IBL` PBR environment-lighting switch.
    pub pbr_environment: PbrEnvironment,
}

impl Default for ViewExtras {
    fn default() -> Self {
        Self {
            viewres_percent: VIEWRES_DEFAULT,
            // Deliberate divergence from AutoCAD's 2D-wireframe initial value:
            // `Shaded` is exactly the filled-mesh + stored-edge display this
            // build had before `VSCURRENT` reached the body representation, so
            // the default is pixel-identical to the pre-token scene.
            visual_style: VisualStyleToken::Shaded,
            // Image-based lighting is the default: the procedural environment is
            // what the PBR shader already renders, so the switch starts on.
            pbr_environment: PbrEnvironment::ImageBased,
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

    /// Store one `VSCURRENT` style token.
    ///
    /// Accepted spellings (case-insensitive, one token):
    ///
    /// * `WIREFRAME`, `2DWIREFRAME`, `2D` → [`VisualStyleToken::Wireframe`];
    /// * `SHADED`, `SHADEDWITHEDGES` → [`VisualStyleToken::Shaded`].
    ///
    /// Every other style is a named refusal (`vscurrent.style_not_implemented`),
    /// never a silent mapping. Session-only: the drawing is not written and no
    /// revision, history entry or `ChangeSet` is produced.
    pub fn set_visual_style(&mut self, style: &str) -> CadResult<()> {
        let token = if style.eq_ignore_ascii_case("WIREFRAME")
            || style.eq_ignore_ascii_case("2DWIREFRAME")
            || style.eq_ignore_ascii_case("2D")
        {
            VisualStyleToken::Wireframe
        } else if style.eq_ignore_ascii_case("SHADED")
            || style.eq_ignore_ascii_case("SHADEDWITHEDGES")
        {
            VisualStyleToken::Shaded
        } else {
            return Err(CadError::Unsupported(format!(
                "vscurrent.style_not_implemented: VSCURRENT accepts WIREFRAME, 2DWIREFRAME, 2D, \
                 SHADED or SHADEDWITHEDGES in this build; `{style}` is NOT IMPLEMENTED and the \
                 visual style was not changed"
            )));
        };
        self.visual_style = token;
        Ok(())
    }

    /// Store one `IBL` environment-lighting value.
    ///
    /// Accepted spellings (case-insensitive, one token):
    ///
    /// * `IBL`, `ON`, `1`, `IMAGEBASED` → [`PbrEnvironment::ImageBased`];
    /// * `LEGACY`, `OFF`, `0` → [`PbrEnvironment::Legacy`].
    ///
    /// Every other value is a named refusal (`ibl.invalid_value`), never a
    /// toggle or a silent fallback. Session-only: the drawing is not written and
    /// no revision, history entry or `ChangeSet` is produced.
    pub fn set_pbr_environment(&mut self, value: &str) -> CadResult<()> {
        let token = value.trim();
        let environment = if token.eq_ignore_ascii_case("IBL")
            || token.eq_ignore_ascii_case("ON")
            || token == "1"
            || token.eq_ignore_ascii_case("IMAGEBASED")
        {
            PbrEnvironment::ImageBased
        } else if token.eq_ignore_ascii_case("LEGACY")
            || token.eq_ignore_ascii_case("OFF")
            || token == "0"
        {
            PbrEnvironment::Legacy
        } else {
            return Err(CadError::Unsupported(format!("ibl.invalid_value: {value}")));
        };
        self.pbr_environment = environment;
        Ok(())
    }
}
