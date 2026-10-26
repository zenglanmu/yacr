//! `$PDMODE` / `$PDSIZE` export round-trip tests.
//!
//! `db1 -> export -> import -> db2` must preserve the drawing-global POINT
//! display style. This is writer/our-reader agreement, not third-party
//! compatibility: nothing here claims AutoCAD or any other consumer draws the
//! same markers from the output.

use super::*;

fn styled_db(mode: i16, size: f64) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.set_point_display_mode(mode);
    builder.set_point_display_size(size);
    builder.finish().unwrap()
}

#[test]
fn the_point_display_style_is_written_and_survives_a_save_open_cycle() {
    // Cross plus a requested surrounding circle, 20% of the viewport height: both
    // values are non-default and the size is negative, so neither can be preserved
    // by accident through the writer's default.
    let db = styled_db(35, -20.0);
    let (bytes, report) = export_db(&db);

    let text = String::from_utf8_lossy(&bytes);
    for variable in ["$PDMODE", "$PDSIZE"] {
        assert!(
            text.contains(variable),
            "{variable} must be in the DXF HEADER section"
        );
    }
    assert!(
        !report.notes.iter().any(|note| note.code.contains("point")),
        "a preserved header value is not a lossy mapping and needs no note: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );

    let reopened = import_bytes(&bytes);
    assert_eq!(
        reopened.point_display_mode(),
        35,
        "the mode survives SAVE->OPEN raw"
    );
    assert_eq!(
        reopened.point_display_size(),
        -20.0,
        "the negative (viewport-percentage) size survives"
    );
}

#[test]
fn the_dwg_defaults_round_trip_as_the_defaults() {
    // A database that never had a style still writes both variables (the writer
    // emits them unconditionally) and reopens at the DWG defaults.
    let (bytes, _report) = export_db(&styled_db(0, 0.0));
    let reopened = import_bytes(&bytes);
    assert_eq!(reopened.point_display_mode(), 0);
    assert_eq!(reopened.point_display_size(), 0.0);
}

#[test]
fn an_absolute_size_keeps_its_magnitude_and_sign() {
    let (bytes, _report) = export_db(&styled_db(3, 2.5));
    let reopened = import_bytes(&bytes);
    assert_eq!(reopened.point_display_mode(), 3, "cross");
    assert_eq!(
        reopened.point_display_size(),
        2.5,
        "an absolute drawing-unit size survives exactly"
    );
}
