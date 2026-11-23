//! Block table identity: imported block ids never use the reserved zero.
//!
//! The `.ymodel` format reserves model id sequence 0 for model space
//! (`cad_domain::ymodel::ModelKey::from_persistent` refuses `BlockModel` seq 0),
//! so an imported block with `BlockId(0)` would make every save of that document
//! write an `elements/` entry its own reader rejects.

use super::*;

/// A synthetic document with two user blocks, each owning one line.
///
/// The fixture is the `block_records` table itself, so no real DWG is needed.
/// The two names sort as PART < PINS, which keeps the "first block" assertion
/// independent of the table's iteration order.
fn document_with_user_blocks() -> acadrust::CadDocument {
    let mut doc = acadrust::CadDocument::new();
    for name in ["PART", "PINS"] {
        let handle = doc.allocate_handle();
        let mut record = acadrust::tables::BlockRecord::new(name);
        record.handle = handle;
        doc.block_records.add(record).expect("add block record");
        let mut line = acadrust::entities::Line::new();
        line.start = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
        line.end = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        line.common.owner_handle = handle;
        doc.add_entity(EntityType::Line(line))
            .expect("add block member");
    }
    doc
}

#[test]
fn imported_block_ids_start_at_one_and_never_use_the_reserved_zero() {
    let doc = document_with_user_blocks();
    let req = request(Vec::new());
    let importer = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let drawing = importer.run().expect("import the synthetic document");
    let db = &drawing.database;

    let ids: Vec<BlockId> = db.blocks().map(|block| block.id).collect();
    assert_eq!(ids.len(), 2, "only the two user blocks are stored: {ids:?}");
    assert!(
        ids.iter().all(|id| id.0 != 0),
        "block id 0 is reserved by the format: {ids:?}"
    );

    // The first imported block gets 1, exactly like the `cad-db` allocator
    // (`next_block_id: 1`), so a save never emits a sequence the reader refuses.
    let part = db.block_by_name("PART").expect("PART is imported");
    assert_eq!(part.id, BlockId(1));
    let pins = db.block_by_name("PINS").expect("PINS is imported");
    assert_ne!(pins.id, part.id);
    // Names and order are unchanged: both fixture names resolve by lookup.
    assert_eq!(
        db.block_by_name("part").map(|block| block.id),
        Some(part.id)
    );
}
