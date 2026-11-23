//! `$INSBASE` / `$LIMMIN` / `$LIMMAX` and `ACAD_SCALELIST` export round-trips.
//!
//! `db1 -> export -> import -> db2` must preserve the values the database holds.
//! This is writer/our-reader agreement, not third-party compatibility: nothing
//! here claims AutoCAD reads the same base point, limits or scale table from the
//! output.

use super::*;

fn header_db() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn the_insertion_base_is_written_and_survives_a_save_open_cycle() {
    let mut db = header_db();
    db.set_insertion_base(p(12.5, -3.25, 2.0), TransactionId(1), "base")
        .expect("a finite base is accepted");
    let (bytes, report) = export_db(&db);

    let variable = header_variable_pairs(&bytes, "$INSBASE");
    assert_eq!(
        variable,
        vec![
            (10, "12.5".to_string()),
            (20, "-3.25".to_string()),
            (30, "2.0".to_string())
        ],
        "$INSBASE must carry the stored point raw"
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code.contains("insbase") || note.code.contains("base")),
        "a preserved header value needs no note: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );

    let reopened = import_bytes(&bytes);
    // The importer of this build does not read `$INSBASE` into `cad-db` yet, so
    // the round-trip is checked at the reader boundary: the acadrust reader sees
    // the value this exporter wrote.
    let acad = acadrust::DxfReader::from_reader(Cursor::new(bytes.clone()))
        .expect("reader")
        .read()
        .expect("the export re-reads");
    assert_eq!(acad.header.model_space_insertion_base.x, 12.5);
    assert_eq!(acad.header.model_space_insertion_base.y, -3.25);
    assert_eq!(acad.header.model_space_insertion_base.z, 2.0);
    // And the database it re-imports into reports the documented absence.
    assert_eq!(
        reopened.insertion_base(),
        None,
        "the import read gap is declared, not fabricated"
    );
}

#[test]
fn both_limit_corners_are_written_and_survive_the_reader() {
    let mut db = header_db();
    db.set_model_limits(
        Some([-10.0, -5.0]),
        Some([420.0, 297.0]),
        TransactionId(1),
        "limits",
    )
    .unwrap();
    let (bytes, report) = export_db(&db);

    assert_eq!(
        header_variable_pairs(&bytes, "$LIMMIN"),
        vec![(10, "-10.0".to_string()), (20, "-5.0".to_string())]
    );
    assert_eq!(
        header_variable_pairs(&bytes, "$LIMMAX"),
        vec![(10, "420.0".to_string()), (20, "297.0".to_string())]
    );
    assert!(
        !report.notes.iter().any(|note| note.code.contains("limits")),
        "a complete pair is not a loss: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );

    let acad = acadrust::DxfReader::from_reader(Cursor::new(bytes))
        .expect("reader")
        .read()
        .expect("the export re-reads");
    assert_eq!(acad.header.model_space_limits_min.x, -10.0);
    assert_eq!(acad.header.model_space_limits_min.y, -5.0);
    assert_eq!(acad.header.model_space_limits_max.x, 420.0);
    assert_eq!(acad.header.model_space_limits_max.y, 297.0);
}

#[test]
fn a_database_without_header_points_writes_the_writer_defaults_without_a_note() {
    // The writer emits all three variables unconditionally; a database that never
    // had a value must not claim one and must not add a loss note (there is
    // nothing this build could have preserved).
    let db = header_db();
    let (bytes, report) = export_db(&db);
    assert_eq!(
        header_variable_pairs(&bytes, "$INSBASE"),
        vec![
            (10, "0.0".to_string()),
            (20, "0.0".to_string()),
            (30, "0.0".to_string())
        ]
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code.contains("insbase") || note.code.contains("limits")),
        "an unstored value is not a conversion loss: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );
}

#[test]
fn a_half_stored_limit_pair_is_declared_in_the_report() {
    let mut db = header_db();
    db.set_model_limits(Some([1.0, 2.0]), None, TransactionId(1), "limits")
        .unwrap();
    let (bytes, report) = export_db(&db);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.limits_half_stored"),
        "a missing corner must be named: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );
    assert_eq!(
        header_variable_pairs(&bytes, "$LIMMIN"),
        vec![(10, "1.0".to_string()), (20, "2.0".to_string())]
    );
    // The unwritten corner is the writer default, exactly as declared.
    assert_eq!(
        header_variable_pairs(&bytes, "$LIMMAX"),
        vec![(10, "0.0".to_string()), (20, "0.0".to_string())]
    );
}

#[test]
fn the_scale_table_round_trips_through_the_export() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_scale(cad_db::Scale {
            id: ScaleId(0),
            name: "1:1".into(),
            paper_units: 1.0,
            drawing_units: 1.0,
        })
        .unwrap();
    builder
        .insert_scale(cad_db::Scale {
            id: ScaleId(1),
            name: "1:100".into(),
            paper_units: 1.0,
            drawing_units: 100.0,
        })
        .unwrap();
    let mut db = builder.finish().unwrap();
    // A row written after import must survive exactly like an imported one.
    db.create_scale("2:1", 2.0, 1.0, TransactionId(3), "scalelist add")
        .expect("a new scale is accepted");

    let (bytes, report) = export_db(&db);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("ACAD_SCALELIST"),
        "the scale dictionary must be written"
    );
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.code.starts_with("export.note.scale")),
        "well-formed rows need no note: {:?}",
        report.notes.iter().map(|n| &n.code).collect::<Vec<_>>()
    );

    let reopened = import_bytes(&bytes);
    let mut rows: Vec<(String, f64, f64)> = reopened
        .scales()
        .map(|scale| (scale.name.clone(), scale.paper_units, scale.drawing_units))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        rows,
        vec![
            ("1:1".to_string(), 1.0, 1.0),
            ("1:100".to_string(), 1.0, 100.0),
            ("2:1".to_string(), 2.0, 1.0),
        ],
        "every named scale survives SAVE->OPEN with its raw pair"
    );
}

#[test]
fn a_database_without_scales_writes_no_scale_dictionary() {
    let db = header_db();
    let (bytes, _report) = export_db(&db);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains("ACAD_SCALELIST"),
        "an empty table must not invent a dictionary"
    );
}
