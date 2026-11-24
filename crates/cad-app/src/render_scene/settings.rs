//! Controller input setters kept out of `mod.rs` (file-size ceiling).
//!
//! These are plain session/derived-display inputs. They are not part of the
//! scene identity (`SceneIdentity`); each is folded into `SceneVersion`, so a
//! change rebuilds the base while an unchanged value stays a no-op.

use super::*;

/// The **session display tokens** that change the derived base without touching
/// `cad-db`.
///
/// Every field is a `cad-app` session token with a real consumer in
/// [`cad_representation::RepresentationContext`]; none of them is drawing data,
/// so none opens a transaction, bumps a revision or records history:
///
/// * [`Self::wipeout_frames`] — `WIPEOUTFRAME` 1/2 versus 0 (the mask boundary
///   frame; the plot half resolves its own value, only `ON` plots it);
/// * [`Self::fill`] — `FILL`: filled hatches/2D solids/wide polylines draw
///   their outline instead of their fill;
/// * [`Self::qtext`] — `QTEXT`: shaped text draws one placeholder rectangle;
/// * [`Self::curve_segments`] — `VIEWRES`: a ceiling on chord segments per
///   curve (`None` keeps the build's own tessellation budget);
/// * [`Self::body_wireframe`] — `VSCURRENT`: `WIREFRAME`/`2DWIREFRAME`/`2D`
///   draw 3D bodies as edge segments only, `SHADED`/`SHADEDWITHEDGES` (the
///   default) keep the filled mesh plus stored edges. Only `Body3d` display
///   changes; every other entity keeps its primitive.
///
/// The struct is `Copy` and compared as a whole in
/// [`SceneVersion`](super::prepared::SceneVersion): one flip rebuilds the base
/// exactly once, and a repeated sync of the same values is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySettings {
    /// Display the WIPEOUT boundary frame (`WIPEOUTFRAME` 1/2 maps to `true`).
    pub wipeout_frames: bool,
    /// Draw filled areas filled (`FILL ON`, AutoCAD's initial value) or
    /// outline-only (`FILL OFF`).
    pub fill: bool,
    /// Draw text as a placeholder rectangle (`QTEXT ON`).
    pub qtext: bool,
    /// How block-attribute values draw (`ATTDISP`).
    ///
    /// A session display token: `Normal` (the default) hides `invisible` values,
    /// `On` shows every value, `Off` hides every non-constant value. A constant
    /// definition draws in every mode. It is part of `SceneVersion`, so a flip
    /// rebuilds the base; it never writes `cad-db`.
    pub attdisp: cad_representation::AttributeDisplay,
    /// `VIEWRES` chord-segment ceiling per curve; `None` = build default.
    pub curve_segments: Option<usize>,
    /// Draw 3D bodies as edge segments only (`VSCURRENT WIREFRAME`).
    ///
    /// `false` is the session default (`SHADED`) and the pre-`VSCURRENT`
    /// display: a body keeps its filled mesh plus stored-edge overlay. This
    /// affects `Body3d` fragments only.
    pub body_wireframe: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            wipeout_frames: false,
            // `FILL` and text drawing start in AutoCAD's documented states, so
            // a controller that never receives a token builds exactly the
            // pre-token scene.
            fill: true,
            qtext: false,
            attdisp: cad_representation::AttributeDisplay::Normal,
            curve_segments: None,
            // The session visual style starts at `SHADED` (this build's
            // deliberate divergence from AutoCAD's 2D-wireframe initial value),
            // which is the pre-`VSCURRENT` filled body display.
            body_wireframe: false,
        }
    }
}

impl DisplaySettings {
    /// The representation context these settings produce.
    pub(crate) fn apply(
        &self,
        context: cad_representation::RepresentationContext,
    ) -> cad_representation::RepresentationContext {
        context
            .with_wipeout_frames(self.wipeout_frames)
            .with_fill(self.fill)
            .with_qtext(self.qtext)
            .with_attdisp(self.attdisp)
            .with_curve_segments(self.curve_segments)
            .with_body_wireframe(self.body_wireframe)
    }
}

impl CadSceneController {
    /// Install the derived vendor snapshot that the next base build must merge.
    ///
    /// The caller replaces the whole snapshot; the controller compares its
    /// revision in [`SceneVersion`] so an unchanged store is free.
    pub fn set_vendor_representations(&mut self, vendor: Option<Arc<VendorRepresentationStore>>) {
        self.vendor = vendor;
    }

    /// Set the display `WIPEOUTFRAME` state (whether each mask draws its
    /// boundary frame). Returns whether the flag changed.
    ///
    /// A change is carried by [`SceneVersion`], so the next preparation rebuilds
    /// the base and the frames appear/disappear without any `cad-db` write. The
    /// plot path resolves its own flag (only `ON` plots a frame), so this only
    /// affects the display base.
    pub fn set_wipeout_frames(&mut self, enabled: bool) -> bool {
        self.set_display_setting(|settings| settings.wipeout_frames = enabled)
    }

    /// Set the session `FILL` token. Returns whether the flag changed.
    ///
    /// `FILL OFF` changes what the base draws (a filled HATCH/2D SOLID/wide
    /// polyline becomes outline-only), so the flip is part of [`SceneVersion`]
    /// and rebuilds the base; it never writes `cad-db`.
    pub fn set_fill(&mut self, enabled: bool) -> bool {
        self.set_display_setting(|settings| settings.fill = enabled)
    }

    /// Set the session `QTEXT` token. Returns whether the flag changed.
    ///
    /// `QTEXT ON` replaces a text entity's glyph fragments with one placeholder
    /// rectangle, so the flip rebuilds the base; it never writes `cad-db`.
    pub fn set_qtext(&mut self, enabled: bool) -> bool {
        self.set_display_setting(|settings| settings.qtext = enabled)
    }

    /// Set the session `ATTDISP` state for block-attribute values.
    ///
    /// A change rebuilds the base (it is part of [`SceneVersion`]) and never
    /// writes `cad-db`. The plot path resolves its own `Normal`, so a screen
    /// `ATTDISP OFF` cannot hide plotted attribute text.
    pub fn set_attdisp(&mut self, mode: cad_representation::AttributeDisplay) -> bool {
        self.set_display_setting(|settings| settings.attdisp = mode)
    }

    /// Set the session `VIEWRES` chord-segment ceiling. Returns whether the
    /// value changed.
    ///
    /// `None` keeps the build's own tessellation budget (the pre-`VIEWRES`
    /// behaviour); `Some(percent)` is the validated session percentage, mapped
    /// through [`cad_representation::viewres_segment_ceiling`] at build time.
    pub fn set_viewres(&mut self, percent: Option<u32>) -> bool {
        self.set_display_setting(|settings| {
            settings.curve_segments = percent.map(cad_representation::viewres_segment_ceiling);
        })
    }

    /// Replace every display token at once. Returns whether anything changed.
    ///
    /// The host's per-tick sync uses this so four independent tokens cannot
    /// schedule four rebuilds for one frame.
    pub fn set_display_settings(&mut self, settings: DisplaySettings) -> bool {
        if self.display == settings {
            return false;
        }
        self.display = settings;
        true
    }

    fn set_display_setting(&mut self, update: impl FnOnce(&mut DisplaySettings)) -> bool {
        let mut settings = self.display;
        update(&mut settings);
        self.set_display_settings(settings)
    }

    /// Bump the font-shaping generation so the next preparation rebuilds the
    /// base with a new font engine.
    pub fn fonts_changed(&mut self) {
        self.fonts_revision += 1;
    }
}
