//! Read-only selection-property rows for vendor-derived entities.
//!
//! A TCH entity is opaque in `cad-db`; the settled vendor snapshot is the only
//! place that knows it has drawable/measurable geometry. These rows are pure
//! metadata over that snapshot, never a second document truth.

use cad_db::DrawingDatabase;

use crate::{PropertyGroup, PropertyRow};

use super::HostController;

impl HostController {
    /// Append `vendor_*` rows when the single selected entity has derived
    /// primitives in the settled snapshot.
    pub(super) fn append_vendor_property_rows(
        &self,
        drawing: &DrawingDatabase,
        properties: &mut crate::SelectionProperties,
    ) {
        if properties.count != 1 {
            return;
        }
        let Some(reference) = self.session.selection.refs().first() else {
            return;
        };
        let Some(entity) = drawing.entity(reference.entity) else {
            return;
        };
        let Some(store) = self.application.vendor_representations.as_deref() else {
            return;
        };
        let Some(primitives) = entity
            .object
            .source_handle
            .as_deref()
            .and_then(|handle| store.for_source_handle(handle))
        else {
            return;
        };
        let mut derived = PropertyRow::new("vendor_derived", "true");
        derived.group = PropertyGroup::Geometry;
        properties.rows.push(derived);
        let mut count = PropertyRow::new("vendor_primitives", primitives.len().to_string());
        count.group = PropertyGroup::Geometry;
        properties.rows.push(count);
        let mut revision = PropertyRow::new("vendor_revision", store.revision().to_string());
        revision.group = PropertyGroup::Geometry;
        properties.rows.push(revision);
    }
}
