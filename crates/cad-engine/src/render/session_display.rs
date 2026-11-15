//! The session **display tokens** the render session forwards to the base build.
//!
//! Split out of `session_impl.rs` for the file-size ceiling. Every setter here
//! is the same shape: it edits one field of
//! [`cad_app::render_scene::DisplaySettings`] and publishes the change through
//! [`RenderSessionHandle::set_display_settings`], which is what makes the token
//! part of `BaseInputIdentity` and the preparation key.
//!
//! None of these writes the drawing: they are session state, so there is no
//! database transaction, revision bump, `cad-history` entry or `ChangeSet`. The
//! *display* really changes, though — `FILL OFF` drops fill geometry, `QTEXT ON`
//! swaps glyph batches for a placeholder box and `VIEWRES` bounds the chord
//! budget — which is why each flip invalidates the published base instead of
//! being a no-op.

use cad_app::render_scene::DisplaySettings;

use super::*;

impl RenderSessionHandle {
    /// Set the display `WIPEOUTFRAME` state (whether each mask draws its
    /// boundary frame). Returns whether the flag changed.
    ///
    /// The flag is part of `BaseInputIdentity` and the preparation key, so a
    /// change invalidates the published base and schedules a re-preparation; an
    /// unchanged value is a no-op. Only the display half lives here — the plot
    /// path resolves `WIPEOUTFRAME 2` separately.
    pub fn set_wipeout_frames(&self, enabled: bool) -> bool {
        let mut settings = self.display();
        settings.wipeout_frames = enabled;
        self.set_display_settings(settings)
    }

    /// Set the session `FILL` token (`false` draws filled areas as outlines).
    /// Returns whether the flag changed.
    ///
    /// Part of `BaseInputIdentity` and the preparation key exactly like
    /// [`Self::set_wipeout_frames`]: `FILL OFF` changes the primitives a base is
    /// made of, so a flip invalidates the published base. Session state only.
    pub fn set_fill(&self, enabled: bool) -> bool {
        let mut settings = self.display();
        settings.fill = enabled;
        self.set_display_settings(settings)
    }

    /// Set the session `QTEXT` token (shaped text draws its placeholder
    /// rectangle instead of its glyphs). Returns whether the flag changed.
    pub fn set_qtext(&self, enabled: bool) -> bool {
        let mut settings = self.display();
        settings.qtext = enabled;
        self.set_display_settings(settings)
    }

    /// Set the session `VIEWRES` percentage (a ceiling on chord segments per
    /// curve). Returns whether the value changed.
    ///
    /// The percentage is mapped through
    /// [`cad_representation::viewres_segment_ceiling`], the same rule the
    /// command validates against, so 1..=1000 here means exactly what the
    /// command's range means. Only chord-tessellated curves are bounded; the
    /// analytic conics are unaffected (see
    /// [`cad_representation::RepresentationContext::curve_segments`]).
    pub fn set_viewres(&self, percent: u32) -> bool {
        let mut settings = self.display();
        settings.curve_segments = Some(cad_representation::viewres_segment_ceiling(percent));
        self.set_display_settings(settings)
    }

    /// Replace every session display token at once. Returns whether anything
    /// changed.
    ///
    /// The host's per-tick sync uses this so four independent tokens cannot
    /// schedule four base rebuilds for one frame.
    pub fn set_display_settings(&self, settings: DisplaySettings) -> bool {
        let mut state = self.0.borrow_mut();
        if state.display == settings {
            return false;
        }
        state.display = settings;
        state.advance_input();
        true
    }

    fn display(&self) -> DisplaySettings {
        self.0.borrow().display
    }
}
