//! The vendor derived-representation snapshot accessor.
//!
//! Split out of `application.rs` for the code-size gate; it is the same
//! `impl Application` block, so the public path is unchanged.

use super::*;

impl Application {
    /// Install or clear the vendor derived-representation snapshot. The render
    /// session keeps its own `Arc`; measurement/query read this copy, and a
    /// replacement document clears it so a stale handle can never measure.
    pub fn set_vendor_representations(
        &mut self,
        store: Option<Arc<crate::vendor::VendorRepresentationStore>>,
    ) {
        self.vendor_representations = store;
    }
}
