//! The Shadows/Fog panels' read: the document's display record.
//!
//! Host content only: the projection reads the current database's
//! `tables/display.json` record and never mutates it. A document without the
//! table answers the documented default, never an error; a carrier with no seam
//! still answers the kind's own refusal at the wire layer.

use cad_domain::display::DisplaySettings;

use super::HostCore;

impl HostCore {
    /// The open document's display settings (the default when none carry).
    pub fn display(&self) -> DisplaySettings {
        let controller = self.controller.borrow();
        controller
            .drawing()
            .map(|database| database.display_settings())
            .unwrap_or_default()
    }
}
