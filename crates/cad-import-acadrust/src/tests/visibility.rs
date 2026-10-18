//! Dynamic-block visibility mapping (spec §3.2).

use super::*;
// ---- Dynamic-block visibility mapping (spec §3.2) ----

use acadrust::objects::{BlockVisibilityParameter, BlockVisibilityState};
use std::collections::BTreeMap;

/// Build a synthetic acadrust document with a user block "DYN" owning three
/// line entities, governed by a two-state visibility parameter.
///
/// Entity 1 is visible, entity 2 is visible, entity 3 is invisible, so state
/// "A" (entities 1+2) is the baked-in active state. The returned handle is the
/// block record handle; the parameter owner chains to it.
/// Insert the three member entities a dynamic-visibility test block references.
///
/// The database builder validates that every block member exists, so the
/// mapping tests must give the definition real entity records.
fn insert_visibility_test_entities(builder: &mut DrawingDatabaseBuilder) {
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for id in [1u128, 2, 3] {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: Some(format!("{id:X}")),
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Block(BlockId(0)),
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: id as f64,
                        y: 0.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: id as f64,
                        y: 1.0,
                        z: 0.0,
                    },
                },
                draw_order: id as i64,
            })
            .unwrap();
    }
}

/// Build a synthetic acadrust document with a user block "DYN" owning three
/// line entities, governed by a two-state visibility parameter.
///
/// Entity 1 is visible, entity 2 is visible, entity 3 is invisible, so state
/// "A" (entities 1+2) is the baked-in active state. The returned handle is the
/// block record handle; the parameter owner chains to it.
fn synthetic_dynamic_document(
    active_is_b: bool,
) -> (
    acadrust::CadDocument,
    acadrust::Handle,
    Vec<acadrust::Handle>,
) {
    let mut doc = acadrust::CadDocument::new();
    let block_handle = doc.allocate_handle();
    let mut record = acadrust::tables::BlockRecord::new("DYN");
    record.handle = block_handle;
    doc.block_records.add(record).expect("add block record");

    let mut handles = Vec::new();
    for i in 0..3u64 {
        let mut line = acadrust::entities::Line::new();
        line.start = acadrust::types::Vector3::new(i as f64, 0.0, 0.0);
        line.end = acadrust::types::Vector3::new(i as f64, 1.0, 0.0);
        // State A ({handles[0], handles[1]}) hides handles[2]; state B
        // ({handles[0], handles[2]}) hides handles[1].
        line.common.invisible = if active_is_b { i == 1 } else { i == 2 };
        line.common.owner_handle = block_handle;
        let handle = doc
            .add_entity(EntityType::Line(line))
            .expect("add block member");
        handles.push(handle);
    }

    let param = BlockVisibilityParameter {
        handle: doc.allocate_handle(),
        owner: block_handle,
        all_blocks: handles.clone(),
        states: vec![
            BlockVisibilityState {
                name: "A".into(),
                visible_blocks: vec![handles[0], handles[1]],
                visible_params: Vec::new(),
            },
            BlockVisibilityState {
                name: "B".into(),
                visible_blocks: vec![handles[0], handles[2]],
                visible_params: Vec::new(),
            },
        ],
        ..BlockVisibilityParameter::default()
    };
    doc.block_visibility_params
        .insert(param.handle, param.clone());
    doc.objects.insert(
        param.handle,
        acadrust::objects::ObjectType::BlockVisibilityParameter(param),
    );
    (doc, block_handle, handles)
}

#[test]
fn importer_records_visibility_states_and_resolves_the_active_state() {
    let (doc, block_handle, handles) = synthetic_dynamic_document(false);
    let req = request(Vec::new());
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &doc, stats, compute_identity(&[]));

    // The importer assigns entity ids in walk order; the mapping keys on the
    // source handle values, so derive them from the synthetic document.
    let member_ids: BTreeMap<u64, EntityId> = [
        (handles[0].value(), EntityId(1)),
        (handles[1].value(), EntityId(2)),
        (handles[2].value(), EntityId(3)),
    ]
    .into_iter()
    .collect();
    // State A is baked in: entities 1 and 2 visible, entity 3 invisible.
    let member_visible: BTreeMap<u64, bool> = [
        (handles[0].value(), true),
        (handles[1].value(), true),
        (handles[2].value(), false),
    ]
    .into_iter()
    .collect();

    importer
        .builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    insert_visibility_test_entities(&mut importer.builder);
    importer.record_dynamic_visibility(block_handle, BlockId(0), &member_ids, &member_visible);

    let db = importer.builder.finish().unwrap();
    let visibility = db
        .block_dynamic_visibility(BlockId(0))
        .expect("a visibility descriptor is recorded");
    assert_eq!(visibility.state_names(), vec!["A", "B"]);
    assert_eq!(visibility.active_state.as_deref(), Some("A"));
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(1), EntityId(2)])
    );
}

#[test]
fn importer_records_state_b_when_its_flags_are_baked_in() {
    let (doc, block_handle, handles) = synthetic_dynamic_document(true);
    let req = request(Vec::new());
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &doc, stats, compute_identity(&[]));
    let member_ids: BTreeMap<u64, EntityId> = [
        (handles[0].value(), EntityId(1)),
        (handles[1].value(), EntityId(2)),
        (handles[2].value(), EntityId(3)),
    ]
    .into_iter()
    .collect();
    // State B: entities 1 and 3 visible, entity 2 invisible.
    let member_visible: BTreeMap<u64, bool> = [
        (handles[0].value(), true),
        (handles[1].value(), false),
        (handles[2].value(), true),
    ]
    .into_iter()
    .collect();
    importer
        .builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    insert_visibility_test_entities(&mut importer.builder);
    importer.record_dynamic_visibility(block_handle, BlockId(0), &member_ids, &member_visible);
    let db = importer.builder.finish().unwrap();
    let visibility = db.block_dynamic_visibility(BlockId(0)).unwrap();
    assert_eq!(visibility.active_state.as_deref(), Some("B"));
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(1), EntityId(3)])
    );
}

#[test]
fn ambiguous_visibility_flags_leave_the_active_state_unknown_and_partial() {
    // Neither state matches the actual flags (entity 1 hidden), so the active
    // state cannot be resolved and must stay unknown, with a stable reason.
    let param = BlockVisibilityParameter {
        all_blocks: vec![acadrust::Handle::new(1), acadrust::Handle::new(2)],
        states: vec![
            BlockVisibilityState {
                name: "A".into(),
                visible_blocks: vec![acadrust::Handle::new(1)],
                visible_params: Vec::new(),
            },
            BlockVisibilityState {
                name: "B".into(),
                visible_blocks: vec![acadrust::Handle::new(2)],
                visible_params: Vec::new(),
            },
        ],
        ..BlockVisibilityParameter::default()
    };
    let member_ids: BTreeMap<u64, EntityId> = [(1u64, EntityId(1)), (2, EntityId(2))]
        .into_iter()
        .collect();
    // Both members are invisible: no state (each makes one visible) matches,
    // so the active state is ambiguous and must stay unknown.
    let member_visible: BTreeMap<u64, bool> = [(1u64, false), (2, false)].into_iter().collect();
    let mapped = map_visibility(&param, &member_ids, &member_visible);
    assert!(mapped.descriptor.active_state.is_none());
    match mapped.completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r == REASON_ACTIVE_UNKNOWN),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    // No active state: every governed member stays visible, never a guess.
    assert!(mapped.descriptor.is_visible(EntityId(1)));
    assert!(mapped.descriptor.is_visible(EntityId(2)));
}

#[test]
fn dangling_visibility_member_handle_is_partial_with_a_stable_reason() {
    let param = BlockVisibilityParameter {
        all_blocks: vec![acadrust::Handle::new(1), acadrust::Handle::new(99)],
        states: vec![BlockVisibilityState {
            name: "A".into(),
            visible_blocks: vec![acadrust::Handle::new(1)],
            visible_params: Vec::new(),
        }],
        ..BlockVisibilityParameter::default()
    };
    // Handle 99 has no imported entity.
    let member_ids: BTreeMap<u64, EntityId> = [(1u64, EntityId(1))].into_iter().collect();
    let member_visible: BTreeMap<u64, bool> = [(1u64, true)].into_iter().collect();
    let mapped = map_visibility(&param, &member_ids, &member_visible);
    match mapped.completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r == REASON_MEMBER_UNRESOLVED),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn ungoverned_block_members_do_not_block_active_state_resolution() {
    // The block owns entity 3, which no state governs. It is visible and must
    // not make the active state ambiguous.
    let param = BlockVisibilityParameter {
        all_blocks: vec![acadrust::Handle::new(1), acadrust::Handle::new(2)],
        states: vec![
            BlockVisibilityState {
                name: "A".into(),
                visible_blocks: vec![acadrust::Handle::new(1)],
                visible_params: Vec::new(),
            },
            BlockVisibilityState {
                name: "B".into(),
                visible_blocks: vec![acadrust::Handle::new(2)],
                visible_params: Vec::new(),
            },
        ],
        ..BlockVisibilityParameter::default()
    };
    let member_ids: BTreeMap<u64, EntityId> = [
        (1u64, EntityId(1)),
        (2, EntityId(2)),
        (3, EntityId(3)), // ungoverned member
    ]
    .into_iter()
    .collect();
    // State A: entity 1 visible, entity 2 hidden; entity 3 (ungoverned) visible.
    let member_visible: BTreeMap<u64, bool> =
        [(1u64, true), (2, false), (3, true)].into_iter().collect();
    let mapped = map_visibility(&param, &member_ids, &member_visible);
    assert_eq!(mapped.descriptor.active_state.as_deref(), Some("A"));
    assert_eq!(mapped.completeness, Completeness::Complete);
    // The ungoverned entity is not part of the governed member set and stays
    // visible under any state.
    assert!(!mapped.descriptor.member_entities.contains(&EntityId(3)));
    assert!(mapped.descriptor.is_visible(EntityId(3)));
}

#[test]
fn importer_resolves_visibility_through_an_insert_reference() {
    let (mut doc, block_handle, handles) = synthetic_dynamic_document(false);
    // A model-space INSERT referencing the dynamic block, as a real evaluated
    // block reference does through its representation data.
    let mut insert = acadrust::entities::Insert::new("DYN", acadrust::types::Vector3::ZERO);
    insert.common.owner_handle = doc.header.model_space_block_handle;
    let insert_handle = doc
        .add_entity(EntityType::Insert(insert))
        .expect("add insert");
    let EntityType::Insert(insert) = doc.get_entity(insert_handle).unwrap().clone() else {
        panic!("just added an insert");
    };

    let req = request(Vec::new());
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &doc, stats, compute_identity(&[]));
    importer
        .builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    insert_visibility_test_entities(&mut importer.builder);
    importer.block_ids.insert("DYN".into(), BlockId(0));
    importer.block_member_ids.insert(
        BlockId(0),
        [
            (handles[0].value(), EntityId(1)),
            (handles[1].value(), EntityId(2)),
            (handles[2].value(), EntityId(3)),
        ]
        .into_iter()
        .collect(),
    );
    importer.block_member_visible.insert(
        BlockId(0),
        [
            (handles[0].value(), true),
            (handles[1].value(), true),
            (handles[2].value(), false),
        ]
        .into_iter()
        .collect(),
    );

    importer.record_insert_dynamic_visibility(&insert);

    let db = importer.builder.finish().unwrap();
    let visibility = db
        .block_dynamic_visibility(BlockId(0))
        .expect("the INSERT path records the descriptor");
    assert_eq!(visibility.state_names(), vec!["A", "B"]);
    assert_eq!(visibility.active_state.as_deref(), Some("A"));
    // The parameter owner is the block record, not the INSERT; the resolver
    // still finds it because the INSERT references that block record.
    assert!(doc.block_visibility_param_for_def(block_handle).is_some());
}
