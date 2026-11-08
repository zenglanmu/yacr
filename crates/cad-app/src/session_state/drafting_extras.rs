//! Session-only drafting flags for the extended command batch.
//!
//! `QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO` and `ISODRAFT` each toggle one flag
//! here. This is **session state only**: no database write, revision bump,
//! `cad-history` entry or `ChangeSet`. This lane wires no renderer/behaviour
//! consumer, so the command handler reports that the effect is not implemented
//! rather than pretending the drawing changed.

/// The five drafting display flags the extended batch can toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DraftingExtras {
    qtext: bool,
    fill: bool,
    tframes: bool,
    regenauto: bool,
    isodraft: bool,
}

impl Default for DraftingExtras {
    fn default() -> Self {
        // AutoCAD's `FILL` starts ON; the rest start OFF in this build.
        Self {
            qtext: false,
            fill: true,
            tframes: false,
            regenauto: false,
            isodraft: false,
        }
    }
}

impl DraftingExtras {
    pub fn qtext(&self) -> bool {
        self.qtext
    }

    pub fn fill(&self) -> bool {
        self.fill
    }

    pub fn tframes(&self) -> bool {
        self.tframes
    }

    pub fn regenauto(&self) -> bool {
        self.regenauto
    }

    pub fn isodraft(&self) -> bool {
        self.isodraft
    }

    pub fn set_qtext(&mut self, value: bool) {
        self.qtext = value;
    }

    pub fn set_fill(&mut self, value: bool) {
        self.fill = value;
    }

    pub fn set_tframes(&mut self, value: bool) {
        self.tframes = value;
    }

    pub fn set_regenauto(&mut self, value: bool) {
        self.regenauto = value;
    }

    pub fn set_isodraft(&mut self, value: bool) {
        self.isodraft = value;
    }

    /// Flip `QTEXT` and return the new state.
    pub fn toggle_qtext(&mut self) -> bool {
        self.qtext = !self.qtext;
        self.qtext
    }

    /// Flip `FILL` and return the new state.
    pub fn toggle_fill(&mut self) -> bool {
        self.fill = !self.fill;
        self.fill
    }

    /// Flip `TFRAMES` and return the new state.
    pub fn toggle_tframes(&mut self) -> bool {
        self.tframes = !self.tframes;
        self.tframes
    }

    /// Flip `REGENAUTO` and return the new state.
    pub fn toggle_regenauto(&mut self) -> bool {
        self.regenauto = !self.regenauto;
        self.regenauto
    }

    /// Flip `ISODRAFT` and return the new state.
    pub fn toggle_isodraft(&mut self) -> bool {
        self.isodraft = !self.isodraft;
        self.isodraft
    }
}
