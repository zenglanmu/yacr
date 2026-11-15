//! `$LTSCALE` export round-trip tests.
//!
//! `db1 -> export -> import -> db2` must preserve the drawing-global linetype
//! scale. This is writer/our-reader agreement, not third-party compatibility:
//! nothing here claims AutoCAD or any other consumer scales dashes identically
//! from the output.

use super::*;

fn scaled_db(scale: Option<f64>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    if let Some(scale) = scale {
        builder
            .set_linetype_scale(scale)
            .expect("a positive finite scale is accepted");
    }
    builder.finish().unwrap()
}

#[test]
fn the_linetype_scale_is_written_and_survives_a_save_open_cycle() {
    let db = scaled_db(Some(2.5));
    let (bytes, report) = export_db(&db);

    // Byte level: `$LTSCALE` is a header variable with one group-40 double
    // carrying the stored value, so a client reading the file sees the scale.
    let variable = header_variable_pairs(&bytes, "$LTSCALE");
    assert_eq!(
        variable,
        vec![(40, "2.5".to_string())],
        "$LTSCALE must be written as group 40 with the stored value"
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code.contains("linetype_scale")),
        "a preserved header value is not a lossy mapping and needs no note: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );

    let reopened = import_bytes(&bytes);
    assert_eq!(
        reopened.linetype_scale(),
        2.5,
        "the scale survives SAVE->OPEN intact"
    );
}

#[test]
fn a_database_that_never_set_the_scale_round_trips_the_dxf_default() {
    // The writer emits `$LTSCALE` unconditionally, so a drawing without an
    // explicit scale must reopen at the DXF default `1.0`, not lose the variable.
    let (bytes, _report) = export_db(&scaled_db(None));
    assert_eq!(
        header_variable_pairs(&bytes, "$LTSCALE"),
        vec![(40, "1.0".to_string())]
    );
    let reopened = import_bytes(&bytes);
    assert_eq!(reopened.linetype_scale(), 1.0);
}

#[test]
fn a_fractional_scale_is_not_rounded() {
    let (bytes, _report) = export_db(&scaled_db(Some(0.125)));
    let variable = header_variable_pairs(&bytes, "$LTSCALE");
    let written: f64 = variable[0].1.parse().expect("group 40 is a double");
    assert_eq!(written, 0.125, "the stored fraction is written exactly");
    let reopened = import_bytes(&bytes);
    assert_eq!(reopened.linetype_scale(), 0.125);
}
