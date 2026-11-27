//! Approximate retained-byte accounting for one retained history step.
//!
//! Split out of `lib.rs` for the file-size gate. The estimate drives the
//! history memory budget; it deliberately over-counts strings (both patch sides
//! are retained), because under-counting would let a large pset or name bypass
//! the budget.

use crate::param_bytes::{expr_bytes, param_row_bytes};
use crate::DrawingHistoryStep;

impl DrawingHistoryStep {
    /// Estimated retained bytes of the whole step (record + every extra batch).
    pub(crate) fn approx_bytes(&self) -> usize {
        let mut bytes = self.record.approx_bytes();
        for patch in &self.linetype_patches {
            bytes += std::mem::size_of::<cad_db::LinetypePatch>();
            for row in patch.before.iter().chain(patch.after.iter()) {
                bytes += std::mem::size_of::<cad_db::LineType>() + row.name.len();
                bytes += row.pattern.elements.len() * std::mem::size_of::<f64>();
            }
        }
        for patch in &self.bim_patches {
            bytes += std::mem::size_of::<cad_db::BimPatch>();
            // Both sides are retained, so both are counted: a property edit
            // keeps the pre-edit pset alongside its replacement.
            bytes += bim_element_bytes(&patch.before) + bim_element_bytes(&patch.after);
        }
        for patch in &self.param_patches {
            bytes += std::mem::size_of::<cad_db::ParamPatch>();
            for row in patch.before.iter().chain(patch.after.iter()) {
                bytes += param_row_bytes(row);
            }
        }
        for patch in &self.param_binding_patches {
            bytes += std::mem::size_of::<cad_db::ParamBindingPatch>();
            for bindings in patch.before.iter().chain(patch.after.iter()) {
                bytes += std::mem::size_of::<cad_domain::param::ParamBindings>();
                for (slot, expr) in bindings.entries() {
                    bytes += slot.len() + expr_bytes(expr);
                }
            }
        }
        bytes += crate::group_patch_bytes::group_patch_bytes(&self.group_patches);
        bytes += self.display_patches.len() * std::mem::size_of::<cad_db::DisplayPatch>();
        for patch in &self.view_patches {
            bytes += std::mem::size_of::<cad_db::ViewPatch>() + patch.name.len();
            for row in patch.before.iter().chain(patch.after.iter()) {
                bytes += std::mem::size_of::<cad_domain::NamedView>() + row.name.len();
                bytes += std::mem::size_of::<cad_domain::ViewCamera>() * 2;
                bytes += std::mem::size_of::<cad_domain::WorkPlane>();
            }
        }
        bytes
    }
}

/// Estimated retained bytes of one element's BIM data.
///
/// The fixed-size value alone would under-count badly: nearly all of a `bim`
/// block is strings (source provenance, pset/property names, `Text` values and
/// each `Real`'s IFC type/unit), and it is exactly those that a pset edit grows.
/// Without them a step could hold an arbitrarily large pset and still report a
/// few hundred bytes, bypassing the history budget.
fn bim_element_bytes(element: &cad_db::BimElement) -> usize {
    let mut bytes = std::mem::size_of::<cad_db::BimElement>();
    bytes += element.source.connector.len();
    for text in [
        element.category.as_deref(),
        element.name.as_deref(),
        element.source.document_id.as_deref(),
        element.source.id.as_deref(),
        element.source.native_id.as_deref(),
        element.level.as_deref(),
        element.element_type.as_deref(),
        element.material.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        bytes += text.len();
    }
    for set in &element.psets {
        bytes += std::mem::size_of::<cad_db::BimPropertySet>() + set.name.len();
        for property in &set.properties {
            bytes += std::mem::size_of::<cad_db::BimProperty>() + property.name.len();
            match &property.value {
                // Empty `Text` is legal, so only its length is counted.
                cad_db::BimValue::Text(text) => bytes += text.len(),
                cad_db::BimValue::Real { ifc_type, unit, .. } => {
                    bytes += ifc_type.as_deref().map_or(0, str::len);
                    bytes += unit.as_deref().map_or(0, str::len);
                }
                cad_db::BimValue::Integer(_)
                | cad_db::BimValue::Bool(_)
                | cad_db::BimValue::Null => {}
            }
        }
    }
    bytes
}
