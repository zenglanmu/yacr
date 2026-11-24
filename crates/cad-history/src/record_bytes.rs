//! Retained-byte estimate for one [`DrawingUndoRecord`].
//!
//! Split out of `lib.rs` for the code-size gate. The estimate is the input to the
//! history memory budget, so it must count every string a batch retains, not just
//! the fixed-size value.

use super::*;

pub(crate) fn approx_bytes(record: &DrawingUndoRecord) -> usize {
    let mut bytes = std::mem::size_of::<DrawingUndoRecord>() + record.label.len();
    for p in &record.patches {
        bytes += std::mem::size_of::<DrawingPatch>();
        if let Some(e) = &p.before {
            bytes += std::mem::size_of::<cad_db::DbEntity>() + e.object.type_key.len();
        }
        if let Some(e) = &p.after {
            bytes += std::mem::size_of::<cad_db::DbEntity>() + e.object.type_key.len();
        }
    }
    for p in &record.block_patches {
        bytes += std::mem::size_of::<cad_db::BlockPatch>();
        for row in p.before.iter().chain(p.after.iter()) {
            bytes += std::mem::size_of::<cad_db::BlockRow>()
                + row.definition.entities.len() * std::mem::size_of::<EntityId>();
            if let Some(metadata) = &row.metadata {
                bytes += metadata.name.len();
            }
        }
    }
    for p in &record.layer_patches {
        bytes += std::mem::size_of::<cad_db::LayerPatch>();
        for row in p.before.iter().chain(p.after.iter()) {
            bytes += std::mem::size_of::<cad_db::LayerRow>() + row.layer.name.len();
            bytes += row.attributes.linetype.len();
            if let Some(plot_style) = &row.attributes.plot_style {
                bytes += plot_style.len();
            }
        }
    }
    for p in &record.layout_patches {
        bytes += std::mem::size_of::<cad_db::LayoutPatch>();
        for row in p.before.iter().chain(p.after.iter()) {
            bytes += std::mem::size_of::<cad_db::LayoutRow>() + row.layout.name.len();
            bytes += row.layout.viewports.len() * std::mem::size_of::<cad_db::PaperViewport>();
            for viewport in &row.layout.viewports {
                bytes += viewport.clip.len() * std::mem::size_of::<Point3>();
                // Per-viewport layer overrides are a map entry each; count
                // the key/value pair so a LAYVPI step is not under-budgeted.
                bytes += viewport.layer_overrides.len() * std::mem::size_of::<(LayerId, bool)>();
            }
            if let Some(plot) = &row.plot {
                bytes += plot.paper_size_name.len();
            }
        }
    }
    for p in &record.style_patches {
        bytes += std::mem::size_of::<cad_db::StylePatch>();
        for row in p.before.iter().chain(p.after.iter()) {
            bytes += std::mem::size_of::<cad_db::Style>() + row.name.len();
            bytes += row
                .resource_keys
                .iter()
                .map(|key| key.len() + std::mem::size_of::<String>())
                .sum::<usize>();
        }
    }
    for p in &record.dim_style_patches {
        bytes += std::mem::size_of::<cad_db::DimStylePatch>();
        for row in p.before.iter().chain(p.after.iter()) {
            bytes += std::mem::size_of::<cad_db::DimStyle>()
                + row.name.len()
                + row.text_style_name.len()
                + row.postfix.len();
        }
    }
    for p in &record.mleader_style_patches {
        bytes += std::mem::size_of::<cad_db::MLeaderStylePatch>();
        for row in p.before.iter().chain(p.after.iter()) {
            bytes += std::mem::size_of::<cad_db::MLeaderStyle>()
                + row.name.len()
                + row.text_style_name.len();
        }
    }
    bytes
}
