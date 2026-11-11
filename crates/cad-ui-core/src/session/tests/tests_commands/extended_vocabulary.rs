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
                // selected table's cell text.
                | ExtendedCommandId::TableCreate
                | ExtendedCommandId::DashTable
                | ExtendedCommandId::TableEdit
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
