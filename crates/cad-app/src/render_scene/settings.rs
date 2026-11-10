//! Controller input setters kept out of `mod.rs` (file-size ceiling).
//!
//! These are plain session/derived-display inputs. They are not part of the
//! scene identity (`SceneIdentity`); each is folded into `SceneVersion`, so a
//! change rebuilds the base while an unchanged value stays a no-op.

use super::*;

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
        if self.wipeout_frames == enabled {
            return false;
        }
        self.wipeout_frames = enabled;
        true
    }

    /// Bump the font-shaping generation so the next preparation rebuilds the
    /// base with a new font engine.
    pub fn fonts_changed(&mut self) {
        self.fonts_revision += 1;
    }
}
