//! Extended-command vocabulary contracts: the `ExtendedCommandId` pilot batch.
//!
//! The wrapper enum is deliberately not part of the main `CommandId` macro
//! list, so these tests prove the two authorities cannot drift: every variant
//! has exactly one `COMMAND_TABLE` row and normalizes back to itself, and the
//! table's `CommandId::Extended(_)` rows are exactly the `ALL` set.

use super::super::super::*;
use cad_app::ExtendedCommandId;

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
        );
        assert_eq!(
            variant.requires_work_mode(),
            expected_work,
            "{selector} work-mode flag"
        );
    }
}

#[test]
fn extended_table_rows_are_exactly_the_all_set() {
    let mut expected: Vec<&'static str> = ExtendedCommandId::ALL
        .iter()
        .map(|variant| variant.selector())
        .collect();
    let mut found: Vec<&'static str> = COMMAND_TABLE
        .iter()
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
