//! Layer attribute export round-trip tests.
//!
//! `db1 -> export -> import -> db2` must preserve every stored layer attribute.
//! This is writer/our-reader agreement, not third-party compatibility: nothing
//! here claims AutoCAD or any other consumer opens the output.

use super::*;
use cad_db::{LayerAttributes, LayerColor, LayerLineWeight};

/// `db1` with three layers that between them exercise every stored field.
fn attributed_db() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    // A real linetype row, so `A-DASHED`'s name resolves instead of being
    // substituted with `Continuous` and reported.
    builder
        .insert_linetype(DbLineType {
            id: LinetypeId(0),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5]),
            complex: false,
        })
        .unwrap();
    for (id, name, visible) in [
        (0u128, "0", true),
        (1, "A-HIDDEN", false),
        (2, "A-DASHED", true),
    ] {
        builder
            .insert_layer(Layer {
                id: LayerId(id),
                name: name.into(),
                visible,
            })
            .unwrap();
    }
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();

    // Layer 0: the documented default (on, thawed, plottable, ACI 7 white).
    builder
        .set_layer_attributes(LayerId(0), LayerAttributes::default())
        .unwrap();
    // A-HIDDEN: off and frozen at once, not plottable, locked.
    builder
        .set_layer_attributes(
            LayerId(1),
            LayerAttributes {
                off: true,
                frozen: true,
                frozen_in_new_viewport: true,
                locked: true,
                xref_dependent: false,
                plottable: false,
                color: LayerColor {
                    aci: Some(5),
                    rgb: [0, 0, 255],
                },
                linetype: "Continuous".into(),
                lineweight: LayerLineWeight::Hundredths(35),
                opacity: 0.8,
                plot_style: None,
            },
        )
        .unwrap();
    // A-DASHED: on and thawed, a named linetype, `Default` weight, opaque.
    builder
        .set_layer_attributes(
            LayerId(2),
            LayerAttributes {
                off: false,
                frozen: false,
                frozen_in_new_viewport: false,
                locked: false,
                xref_dependent: false,
                plottable: true,
                color: LayerColor {
                    aci: Some(2),
                    rgb: [255, 255, 0],
                },
                linetype: "DASHED".into(),
                lineweight: LayerLineWeight::Default,
                opacity: 1.0,
                plot_style: None,
            },
        )
        .unwrap();
    builder.finish().unwrap()
}

/// Every `LAYER` record of the exported TABLES section as `(group, value)`
/// pairs, in file order.
///
/// A group-scan over the ASCII pairs, in the same style as the DIMENSION helper
/// in `dimension.rs`: it reads what the file actually says rather than trusting
/// the round trip alone.
fn layer_records(bytes: &[u8]) -> Vec<Vec<(i32, String)>> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim() == "0" && lines[i + 1].trim() == "LAYER" {
            let mut pairs = Vec::new();
            let mut j = i + 2;
            while j + 1 < lines.len() {
                let code = lines[j].trim();
                if code == "0" {
                    break;
                }
                let group: i32 = code.parse().expect("DXF group code is an integer");
                pairs.push((group, lines[j + 1].trim().to_string()));
                j += 2;
            }
            records.push(pairs);
        }
        i += 1;
    }
    records
}

/// The first value of `group` in a record.
fn group(record: &[(i32, String)], group: i32) -> Option<&str> {
    record
        .iter()
        .find(|(code, _)| *code == group)
        .map(|(_, value)| value.as_str())
}

/// The record of the layer named `name`.
fn record_named(records: &[Vec<(i32, String)>], name: &str) -> Vec<(i32, String)> {
    records
        .iter()
        .find(|record| group(record, 2) == Some(name))
        .unwrap_or_else(|| panic!("no LAYER record named {name}"))
        .clone()
}

#[test]
fn every_layer_attribute_survives_an_export_import_round_trip() {
    let db1 = attributed_db();
    let (bytes, report) = export_db(&db1);
    let db2 = import_bytes(&bytes);

    for layer in db1.layers() {
        let before = db1.layer_attributes(layer.id);
        let after = db2.layer_attributes(
            db2.layers()
                .find(|candidate| candidate.name == layer.name)
                .unwrap_or_else(|| panic!("layer '{}' did not come back", layer.name))
                .id,
        );
        assert_eq!(
            after, before,
            "layer '{}' attributes changed across the round trip",
            layer.name
        );
        // And the invariant still holds on the re-imported row.
        let row = db2
            .layers()
            .find(|candidate| candidate.name == layer.name)
            .expect("row exists");
        assert_eq!(row.visible, !(after.off || after.frozen));
    }

    // Effective visibility is unchanged for every layer, which is the output
    // compatibility this phase promises.
    for layer in db1.layers() {
        let after = db2
            .layers()
            .find(|candidate| candidate.name == layer.name)
            .expect("row exists");
        assert_eq!(after.visible, layer.visible, "layer '{}'", layer.name);
    }

    // Nothing was dropped from the layer table, so no layer diagnostic fires.
    let layer_codes: Vec<&str> = report
        .notes
        .iter()
        .map(|note| note.code.as_str())
        .filter(|code| code.starts_with("export.layer_"))
        .collect();
    assert!(layer_codes.is_empty(), "unexpected notes: {layer_codes:?}");
}

#[test]
fn exported_layer_records_carry_the_expected_group_codes() {
    let (bytes, report) = export_db(&attributed_db());
    let records = layer_records(&bytes);
    assert_eq!(records.len(), 3, "one record per layer row");
    assert!(report
        .notes
        .iter()
        .all(|note| !note.code.starts_with("export.layer_")));

    // Layer 0: default weight is `ByLayer` (-1), ACI 7, plottable (290 = 1),
    // neither frozen (70 bit 1) nor off (positive 62 sign).
    let zero = record_named(&records, "0");
    assert_eq!(group(&zero, 70), Some("0"));
    assert_eq!(group(&zero, 62), Some("7"));
    assert_eq!(group(&zero, 6), Some("Continuous"));
    assert_eq!(group(&zero, 370), Some("-1"));
    assert_eq!(group(&zero, 290), Some("1"));

    // A-HIDDEN: off and frozen are written as **distinct** encodings — group 70
    // bit 1 for frozen (plus bit 2 for the new-viewport flag) and a **negative**
    // group 62 for off. `290` is the bool type, written as `0`.
    let hidden = record_named(&records, "A-HIDDEN");
    // 1 (frozen) | 2 (frozen in new viewport) | 4 (locked) = 7.
    assert_eq!(group(&hidden, 70), Some("7"));
    assert_eq!(group(&hidden, 62), Some("-5"));
    assert_eq!(group(&hidden, 370), Some("35"));
    assert_eq!(group(&hidden, 290), Some("0"));

    // A-DASHED: a named linetype plus the `Default` weight (-3), on and
    // unfrozen, hence a positive colour index.
    let dashed = record_named(&records, "A-DASHED");
    assert_eq!(group(&dashed, 70), Some("0"));
    assert_eq!(group(&dashed, 62), Some("2"));
    assert_eq!(group(&dashed, 6), Some("DASHED"));
    assert_eq!(group(&dashed, 370), Some("-3"));
    assert_eq!(group(&dashed, 290), Some("1"));
}

#[test]
fn transparency_is_written_inverted_on_the_source_byte_scale() {
    // The stored value is opacity (1 = opaque); the file stores transparency
    // (0 = opaque), so 0.8 opacity must come back as an `AcCmTransparency`
    // XDATA integer whose low byte is 51.
    let (bytes, _) = export_db(&attributed_db());
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut found = None;
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim() == "0" && lines[i + 1].trim() == "LAYER" {
            let mut name = None;
            let mut j = i + 2;
            while j + 1 < lines.len() {
                let code = lines[j].trim();
                if code == "0" {
                    break;
                }
                if code == "2" && name.is_none() {
                    name = Some(lines[j + 1].trim().to_string());
                }
                if code == "1071" && name.as_deref() == Some("A-HIDDEN") {
                    found = Some(lines[j + 1].trim().parse::<i32>().expect("group 1071"));
                }
                j += 2;
            }
        }
        i += 1;
    }
    let packed = found.expect("A-HIDDEN carries an AcCmTransparency value");
    // Packed form: method byte 2 in the high byte, `255 - alpha` in the low one.
    assert_eq!((packed >> 24) as u8, 2, "packed transparency method byte");
    assert_eq!((packed & 0xFF) as u8, 255 - 51);
}

#[test]
fn a_stored_plot_style_name_is_dropped_with_a_diagnostic() {
    // A plot-style *name* cannot be written: both writers emit the plot-style
    // handle (group 390), never the name. The drop is explicit.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(0),
            LayerAttributes {
                plot_style: Some("Normal".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let db = builder.finish().unwrap();

    let (bytes, report) = export_db(&db);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.layer_plot_style_dropped"),
        "expected a drop diagnostic, got {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );
    // The re-imported layer has no name to restore — the tolerated delta.
    let reimported = import_bytes(&bytes);
    assert_eq!(
        reimported.layer_attributes(LayerId(0)).plot_style,
        None,
        "the name cannot survive export"
    );
}

#[test]
fn an_undefined_linetype_name_is_substituted_with_a_diagnostic() {
    // A layer naming a linetype this drawing does not define must not become a
    // dangling reference: it is written as `Continuous` and reported.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(0),
            LayerAttributes {
                linetype: "GHOST".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let db = builder.finish().unwrap();

    let (bytes, report) = export_db(&db);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.layer_linetype_substituted"),
        "expected a substitution diagnostic, got {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );
    let records = layer_records(&bytes);
    let zero = record_named(&records, "0");
    assert_eq!(group(&zero, 6), Some("Continuous"));
    // No dangling reference was created either.
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("GHOST"));
}

#[test]
fn a_true_colour_layer_keeps_its_rgb_and_reports_no_index() {
    // `aci = None` is written as a true colour (group 420), so the RGB value
    // survives while there is no index to lose. The tolerated delta of a
    // true-colour source is therefore the *approximate* index the DXF writer
    // also emits in group 62, which the reader lets group 420 override.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(0),
            LayerAttributes {
                color: LayerColor {
                    aci: None,
                    rgb: [10, 20, 30],
                },
                ..Default::default()
            },
        )
        .unwrap();
    let db = builder.finish().unwrap();

    let (bytes, _) = export_db(&db);
    let reimported = import_bytes(&bytes);
    let attributes = reimported.layer_attributes(LayerId(0));
    assert_eq!(attributes.color.aci, None);
    assert_eq!(attributes.color.rgb, [10, 20, 30]);
}
