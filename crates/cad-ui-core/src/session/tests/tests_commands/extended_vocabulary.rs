//! Extended-command vocabulary contracts: the `ExtendedCommandId` pilot batch.
//!
//! The wrapper enum is deliberately not part of the main `CommandId` macro
//! list, so these tests prove the two authorities cannot drift: every variant
//! has exactly one `COMMAND_TABLE` row and normalizes back to itself, and the
//! table's `CommandId::Extended(_)` rows are exactly the `ALL` set.

use super::super::super::*;
use cad_app::{ExtendedCommandId, RefusalId};

#[test]
fn every_extended_variant_has_one_table_row_and_normalizes() {
    for &variant in ExtendedCommandId::ALL {
        let selector = variant.selector();
        let rows: Vec<_> = COMMAND_TABLE
            .iter()
            .filter(|(name, _)| *name == selector)
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "{selector} must appear exactly once in COMMAND_TABLE"
        );
        assert_eq!(
            rows[0].1,
            CommandId::Extended(variant),
            "{selector} must map to its ExtendedCommandId variant"
        );
        assert_eq!(
            normalize_command(selector),
            Some(CommandId::Extended(variant)),
            "{selector}"
        );
        assert_eq!(canonical_command_name(selector), Some(selector));
        // Waves 1–3 are Viewer-legal. Wave 4's twelve entity-writing commands
        // declare Work mode; every other variant stays Viewer-legal.
        let expected_work = matches!(
            variant,
            ExtendedCommandId::CenterMark
                | ExtendedCommandId::CenterLine
                | ExtendedCommandId::NCopy
                | ExtendedCommandId::CopyToLayer
                | ExtendedCommandId::DashCopyToLayer
                | ExtendedCommandId::CDOrder
                | ExtendedCommandId::Flatten
                | ExtendedCommandId::XPlode
                | ExtendedCommandId::BTrim
                | ExtendedCommandId::BExtend
                | ExtendedCommandId::Change
                | ExtendedCommandId::ChProp
                | ExtendedCommandId::Array
                | ExtendedCommandId::DashArray
                | ExtendedCommandId::ArrayClassic
                | ExtendedCommandId::Blend
                | ExtendedCommandId::BreakLine
                | ExtendedCommandId::DashBlock
                | ExtendedCommandId::DashBoundary
                | ExtendedCommandId::DashInsert
                | ExtendedCommandId::DashHatch
                | ExtendedCommandId::DashHatchEdit
                | ExtendedCommandId::ExOffset
                | ExtendedCommandId::LayerStateSave
                | ExtendedCommandId::MView
                | ExtendedCommandId::PageSetup
                | ExtendedCommandId::DashPageSetup
                | ExtendedCommandId::PageSetupIn
                | ExtendedCommandId::DashPageSetupIn
                // Wave 16 raster-image authoring writes entity rows (attach) or
                // rewrites one selected image's clip or display transfer.
                | ExtendedCommandId::ImageAttach
                | ExtendedCommandId::ImageClip
                | ExtendedCommandId::ImageAdjust
                | ExtendedCommandId::DashImageAdjust
                // TABLE / -TABLE insert one table row; TABLEDIT rewrites one
                // selected table's cell text or its merge spans; MATCHCELL copies
                // a cell style and TABLESTYLE rewrites the table's style snapshot.
                // TABLEEXPORT writes a file, never the drawing, so it stays
                // Viewer-legal like SAVE/PLOT.
                | ExtendedCommandId::TableCreate
                | ExtendedCommandId::DashTable
                | ExtendedCommandId::TableEdit
                | ExtendedCommandId::MatchCell
                | ExtendedCommandId::TableStyleList
                // Wave 26 annotation/paper-space writes that landed before this
                // contract list was extended (TXTEXP/ARCTEXT insert rows; the
                // page/paper writes rewrite layout or entity rows).
                | ExtendedCommandId::TextExplode
                | ExtendedCommandId::ArcText
                | ExtendedCommandId::MvSetup
                | ExtendedCommandId::PsbScale
                | ExtendedCommandId::PstScale
                | ExtendedCommandId::AlignSpace
                | ExtendedCommandId::VpSync
                // Wave 27: SHP2BLK defines blocks and rewrites entity rows,
                // MINSERT inserts block references, ADDSELECTED inserts one
                // entity. Every one is an entity/block-table write.
                | ExtendedCommandId::Shp2Blk
                | ExtendedCommandId::MInsert
                | ExtendedCommandId::AddSelected
                // 2026-11-12: the MULTILEADER edits, the TOLERANCE frame and the
                // MLEDIT two-LINE subset write entity rows.
                | ExtendedCommandId::MLeaderAlign
                | ExtendedCommandId::MLeaderEdit
                | ExtendedCommandId::MLeaderCollect
                | ExtendedCommandId::Tolerance
                | ExtendedCommandId::MlEdit
                | ExtendedCommandId::DashMlEdit
                // XCLIP writes one INSERT display attribute (the block-local
                // clip boundary) through the display-transaction path;
                // CLIP/CLIPIT write the same image/block clip models.
                | ExtendedCommandId::Xclip
                | ExtendedCommandId::Clip
                | ExtendedCommandId::ClipIt
                // Wave 30/31: the paper-space CHSPACE move and the two
                // graduated vector-header writes ($INSBASE, $LIMMIN/$LIMMAX).
                | ExtendedCommandId::ChSpace
                | ExtendedCommandId::BasePoint
                | ExtendedCommandId::DrawingLimits
                // ADR-0036 C1/C2: the seven 3D primitive creates and the three
                // solid booleans write entity rows (booleans run as a job but
                // still mutate the drawing).
                | ExtendedCommandId::Box
                | ExtendedCommandId::Cylinder
                | ExtendedCommandId::Sphere
                | ExtendedCommandId::Cone
                | ExtendedCommandId::Torus
                | ExtendedCommandId::Wedge
                | ExtendedCommandId::Pyramid
                | ExtendedCommandId::Union
                | ExtendedCommandId::Subtract
                | ExtendedCommandId::Intersect
                // ADR-0036 P2.0: CHAMFEREDGE rewrites one exact body in place;
                // PRESSPULL (2026-11-25) moves one face of it in place; EXTRUDE
                // inserts a new body from the selected profile.
                | ExtendedCommandId::ChamferEdge
                | ExtendedCommandId::PressPull
                | ExtendedCommandId::Extrude
                // XEDGES inserts one LINE per stored display-edge segment.
                | ExtendedCommandId::Xedges
                // ADR-0055 M3: block-attribute definitions and value edits
                // rewrite definition members / INSERT rows; BURST creates text
                // rows and consumes the references.
                | ExtendedCommandId::AttDef
                | ExtendedCommandId::DashAttDef
                | ExtendedCommandId::AttEdit
                | ExtendedCommandId::DashAttEdit
                | ExtendedCommandId::AttIpEdit
                | ExtendedCommandId::EAttEdit
                | ExtendedCommandId::Gatte
                | ExtendedCommandId::AttRedef
                | ExtendedCommandId::AttSync
                | ExtendedCommandId::BattOrder
                | ExtendedCommandId::Burst
                // DIMREASSOC clears a dimension text override (a drawing
                // write); DIMREASSOCIATE stays a payload-free report.
                | ExtendedCommandId::DimReassoc
                // ADR-0061 P-MESH-1b/2/3: repair/rebuild/refine/smooth/cap all
                // rewrite the selected mesh row (one transaction, one undo).
                | ExtendedCommandId::MeshRepair
                | ExtendedCommandId::MeshRebuild
                | ExtendedCommandId::MeshRefine
                | ExtendedCommandId::MeshSmooth
                | ExtendedCommandId::MeshCap
        );
        assert_eq!(
            variant.requires_work_mode(),
            expected_work,
            "{selector} work-mode flag"
        );
    }

    // Waves 17-24: one sibling-table row per `RefusalId`, all Viewer-legal.
    for &name in RefusalId::ALL {
        let selector = name.selector();
        let rows: Vec<_> = REFUSAL_TABLE
            .iter()
            .filter(|(candidate, _)| *candidate == selector)
            .collect();
        assert_eq!(rows.len(), 1, "{selector} must appear exactly once");
        assert_eq!(
            rows[0].1,
            CommandId::Extended(ExtendedCommandId::Refusal(name)),
            "{selector}"
        );
        assert_eq!(
            normalize_command(selector),
            Some(CommandId::Extended(ExtendedCommandId::Refusal(name))),
            "{selector}"
        );
        assert_eq!(canonical_command_name(selector), Some(selector));
        assert_eq!(name.selector(), selector);
        assert!(name.reason().starts_with(&selector.to_ascii_lowercase()));
        assert!(!name.reason().is_empty());
        assert!(
            !ExtendedCommandId::Refusal(name).requires_work_mode(),
            "{selector} must stay Viewer-legal"
        );
    }
}

#[test]
fn extended_table_rows_are_exactly_the_all_set() {
    let mut expected: Vec<&'static str> = ExtendedCommandId::ALL
        .iter()
        .map(|variant| variant.selector())
        .collect();
    expected.extend(RefusalId::ALL.iter().map(|name| name.selector()));
    let mut found: Vec<&'static str> = COMMAND_TABLE
        .iter()
        .chain(REFUSAL_TABLE.iter())
        .filter(|(_, id)| matches!(id, CommandId::Extended(_)))
        .map(|(name, _)| *name)
        .collect();
    expected.sort_unstable();
    found.sort_unstable();
    assert_eq!(
        found, expected,
        "the CommandId::Extended rows must be exactly the ExtendedCommandId::ALL set"
    );
}
