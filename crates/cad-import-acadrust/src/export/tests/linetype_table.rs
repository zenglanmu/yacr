//! Linetype **table** round-trip: a row created by `LINETYPE CREATE` is written
//! into the DXF `LTYPE` table and read back by the importer.
//!
//! The table pass ([`super::super::tables::write_tables`]) already iterates every
//! stored row — including one nothing references yet — so this test pins that the
//! created row's **name, element list and cycle** survive SAVE → OPEN, not just a
//! row created as a side effect of an explicit entity linetype.
//!
//! Internal consistency only: this is our writer read by our reader, not a claim
//! that any third-party DXF consumer accepts the output.

use super::*;
use cad_db::LayerAttributes;

#[test]
fn a_created_linetype_row_exports_and_reimports_with_its_pattern() {
    let mut db = db_with(Vec::new());
    let (id, _) = db
        .create_linetype(
            "CENTER",
            &[2.0, -0.5, 0.5, -0.5],
            TransactionId(1),
            "create linetype",
        )
        .expect("the row is created");
    // Reference the new row from layer 0, so the exported layer record also names
    // it and the re-import has to resolve the name against the table.
    db.set_layer_attributes(
        LayerId(0),
        LayerAttributes {
            linetype: "CENTER".into(),
            ..LayerAttributes::default()
        },
        TransactionId(2),
        "fixture layer linetype",
    )
    .expect("layer 0 takes the new linetype");

    let (bytes, report) = export_db(&db);
    assert_eq!(report.format, ExportFormat::DxfText);

    let reimported = import_bytes(&bytes);
    let row = reimported
        .linetypes()
        .find(|row| row.name == "CENTER")
        .expect("the created linetype is in the exported table");
    assert_eq!(
        row.pattern.elements,
        vec![2.0, -0.5, 0.5, -0.5],
        "group-49 element list (positive dash, negative gap) survives"
    );
    assert_eq!(
        row.pattern.cycle, 3.5,
        "the cycle is re-derived from the same elements"
    );
    assert!(!row.complex, "a created row carries no shape/text elements");
    let stored = reimported
        .linetype(row.id)
        .expect("the row is reachable by its re-imported id");
    assert_eq!(stored.name, "CENTER");
    let _ = id;

    assert_eq!(
        reimported.layer_attributes(LayerId(0)).linetype,
        "CENTER",
        "the layer's linetype name resolves to the exported table row"
    );
}

#[test]
fn every_stored_linetype_row_is_written_even_when_no_entity_uses_it() {
    let mut db = db_with(Vec::new());
    db.create_linetype("CENTER", &[2.0, -0.5], TransactionId(1), "create")
        .unwrap();
    db.create_linetype(
        "PHANTOM",
        &[3.0, -1.0, 0.25, -1.0],
        TransactionId(2),
        "create",
    )
    .unwrap();
    let expected: Vec<(String, Vec<f64>)> = db
        .linetypes()
        .map(|row| (row.name.clone(), row.pattern.elements.clone()))
        .collect();
    assert_eq!(expected.len(), 2, "the two created rows");

    let (bytes, _) = export_db(&db);
    let reimported = import_bytes(&bytes);
    for (name, elements) in expected {
        let row = reimported
            .linetypes()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("linetype '{name}' is exported"));
        assert_eq!(row.pattern.elements, elements, "{name}");
    }
}
