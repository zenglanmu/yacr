//! MTEXT line-spacing (DXF groups 44 + 73) export/import round trip.
//!
//! Writer + our reader agreement is internal consistency, not third-party
//! compatibility: nothing here claims AutoCAD or any other consumer opens the
//! output.

use super::*;

use super::entities::{export_one_as, export_text_as};

/// DXF group-code/value pairs of a text export, in file order.
///
/// The text writer emits `code\r\nvalue\r\n` pairs, so splitting on the line
/// breaks and pairing them is enough to inspect what the writer actually wrote
/// (no re-parsing through our own reader).
fn dxf_pairs(bytes: &[u8]) -> Vec<(i32, String)> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    lines
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|pair| {
            let code: i32 = pair[0].trim().parse().ok()?;
            Some((code, pair[1].trim().to_string()))
        })
        .collect()
}

/// Every value of one DXF group code written inside the entity record that
/// starts at `marker` (a subclass marker such as `AcDbMText`) and ends at the
/// next group-0 record boundary.
///
/// Scoping matters: groups 73/44 also occur in unrelated records (layouts,
/// styles), so a whole-file scan would not be evidence about this entity.
fn entity_group_values(bytes: &[u8], marker: &str, code: i32) -> Vec<String> {
    let pairs = dxf_pairs(bytes);
    let Some(start) = pairs
        .iter()
        .position(|(group, value)| *group == 100 && value == marker)
    else {
        panic!("no {marker} subclass marker in the export");
    };
    pairs[start..]
        .iter()
        .skip(1)
        .take_while(|(group, _)| *group != 0)
        .filter(|(group, _)| *group == code)
        .map(|(_, value)| value.clone())
        .collect()
}

#[test]
fn mtext_line_spacing_defaults_write_the_dxf_groups_unchanged() {
    // A source without line-spacing data carries `1.0 / at least`; the writer
    // has always emitted groups 73 and 44 for MTEXT, so the bytes are what they
    // were before the field existed: one pair each, at the DXF defaults.
    let (bytes, report) = export_text_as("AcDbMText", "a\\Pb", None);
    assert_eq!(report.counts.exact, 1, "{report:?}");
    assert_eq!(
        entity_group_values(&bytes, "AcDbMText", 73),
        vec!["1".to_string()]
    );
    assert_eq!(
        entity_group_values(&bytes, "AcDbMText", 44),
        vec!["1.0".to_string()],
        "the DXF real default, exactly as the writer always wrote it"
    );

    // A full export → import → export round trip keeps the same two groups, so
    // the neutral default never drifts into a non-default value.
    let reimported = import_bytes(&bytes);
    let (again, _) = export_db(&reimported);
    assert_eq!(
        entity_group_values(&again, "AcDbMText", 73),
        vec!["1".to_string()]
    );
    assert_eq!(
        entity_group_values(&again, "AcDbMText", 44),
        vec!["1.0".to_string()]
    );
}

#[test]
fn mtext_line_spacing_groups_44_and_73_round_trip() {
    let spacing = cad_domain::TextLineSpacing {
        factor: 1.75,
        style: cad_domain::TextLineSpacingStyle::Exactly,
    };
    let (bytes, report) = export_one_as(
        "AcDbMText",
        SemanticGeometry::Text {
            text: "a\\Pb".into(),
            position: p(1.0, 2.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Top,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: spacing,
        },
    );
    // Writing the stored spacing is faithful to the database: still `Exact`.
    assert_eq!(report.counts.exact, 1, "{report:?}");
    assert_eq!(
        entity_group_values(&bytes, "AcDbMText", 44),
        vec!["1.75".to_string()]
    );
    assert_eq!(
        entity_group_values(&bytes, "AcDbMText", 73),
        vec!["2".to_string()]
    );
    match &read_entities(&bytes)[0] {
        EntityType::MText(mtext) => {
            assert_eq!(mtext.line_spacing_factor, 1.75, "group 44");
            assert_eq!(
                mtext.line_spacing_style,
                acadrust::entities::LineSpacingStyle::Exactly,
                "group 73"
            );
        }
        other => panic!("expected MTEXT, got {other:?}"),
    }

    // Our own importer carries both halves into the domain model.
    let reimported = import_bytes(&bytes);
    let stored = reimported
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { line_spacing, .. } => Some(*line_spacing),
            _ => None,
        })
        .expect("the MTEXT reimports");
    assert_eq!(stored, spacing, "the factor and the style both survive");
}
