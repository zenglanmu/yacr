use super::edit::dirty_entities;
use super::*;
use crate::{host::HostController, Command, CommandId, CommandPayload};
use cad_db::{
    BlockDefinition, ChangeMask, ChangeSet, DbEntity, DbObject, DrawingDatabaseBuilder, Layer,
    ObjectChange,
};
use cad_domain::{
    BlockId, DatabaseId, EntityId, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId,
    TransactionId, Transform3, ViewportId,
};
use cad_resources::ResourceKey;
use cad_scene::{ImageBatch, MaskBatch};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// Run one command and return `(pre-commit snapshot, post-commit snapshot, the
/// authoritative changeset the commit published)`.
fn commit(
    host: &mut HostController,
    id: CommandId,
    payload: CommandPayload,
) -> (Arc<DrawingDatabase>, Arc<DrawingDatabase>, Arc<ChangeSet>) {
    let before = host.drawing().unwrap();
    let traced = host
        .execute_traced(Command {
            schema_version: 1,
            id,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload,
        })
        .unwrap();
    let outcome = traced.result.unwrap();
    let changes = outcome
        .changes
        .clone()
        .expect("a committed create publishes a ChangeSet");
    (before, host.drawing().unwrap(), changes)
}

fn line_points() -> CommandPayload {
    CommandPayload::Points(vec![p(2.0, 3.0), p(4.0, 5.0)])
}

fn transition() -> (Arc<DrawingDatabase>, Arc<DrawingDatabase>, Arc<ChangeSet>) {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    commit(&mut host, CommandId::CreateLine, line_points())
}

fn prepare_fonts(
    controller: &mut CadSceneController,
    db: Arc<DrawingDatabase>,
    fonts: Option<Arc<FontEngine>>,
    edit: Option<&DrawingEditReceipt>,
) {
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(db),
            DocumentId(1),
            fonts,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            edit,
        )
        .unwrap();
}

fn prepare(
    controller: &mut CadSceneController,
    db: Arc<DrawingDatabase>,
    edit: Option<&DrawingEditReceipt>,
) {
    prepare_fonts(controller, db, None, edit);
}

/// A test font engine with the repository's open font as the fallback, so a
/// created TEXT really shapes into batches. Without it both paths would still
/// agree, but on the unshaped placeholder — a vacuous comparison.
fn test_fonts() -> Arc<FontEngine> {
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    fonts.set_fallback(vec!["osifont.ttf".into()]);
    Arc::new(fonts)
}

#[test]
fn line_append_matches_full_assembly_and_retains_predecessor_across_overlay() {
    let (before, after, changes) = transition();
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes).unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    let predecessor = controller.ready.as_ref().unwrap().base.clone();
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    let append = ready.base_append.as_ref().expect("authorized append");
    assert!(append.predecessor.same_as(&predecessor));
    assert_eq!(append.keep_batches, predecessor.batch_len());
    let full = build_scene(&after, ready.base.stamp().clone()).unwrap();
    assert_eq!(ready.base.to_delta().added, full.added);
    assert_eq!(ready.base.images(), full.images);
    assert_eq!(ready.base.masks(), full.masks);
    let mut overlays = OverlayInputs::default();
    overlays.visibility.axes = !overlays.visibility.axes;
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(after),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &overlays,
            None,
            Some(&receipt),
        )
        .unwrap();
    assert!(controller
        .ready
        .as_ref()
        .unwrap()
        .base_append
        .as_ref()
        .unwrap()
        .predecessor
        .same_as(&predecessor));
}

#[test]
fn line_append_rejects_wrong_predecessor_and_font_changes() {
    for font_change in [false, true] {
        let (before, after, changes) = transition();
        let receipt =
            DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes).unwrap();
        let mut controller = CadSceneController::default();
        let adopted = if font_change {
            before
        } else {
            Arc::new(before.as_ref().clone())
        };
        prepare(&mut controller, adopted, None);
        if font_change {
            controller.fonts_changed();
        }
        prepare(&mut controller, after.clone(), Some(&receipt));
        let ready = controller.ready.as_ref().unwrap();
        assert!(ready.base_append.is_none());
        assert_eq!(
            ready.base.to_delta().added,
            build_scene(&after, ready.base.stamp().clone())
                .unwrap()
                .added
        );
    }
}

#[test]
fn non_line_single_entity_inserts_take_the_append_path() {
    let fonts = test_fonts();
    let cases: Vec<(&str, CommandId, CommandPayload)> = vec![
        (
            "circle",
            CommandId::CreateCircle,
            CommandPayload::Points(vec![p(10.0, 10.0), p(30.0, 10.0)]),
        ),
        (
            "open lwpolyline",
            CommandId::CreatePolyline,
            CommandPayload::Points(vec![p(1.0, 1.0), p(2.0, 4.0), p(6.0, 2.0)]),
        ),
        (
            "closed lwpolyline (RECTANG)",
            CommandId::CreateRectangle,
            CommandPayload::Points(vec![p(20.0, 20.0), p(60.0, 45.0)]),
        ),
        (
            "text",
            CommandId::CreateText,
            CommandPayload::Text {
                position: p(100.0, 100.0),
                text: "YACR".into(),
                height: 25.0,
            },
        ),
    ];
    for (label, id, payload) in cases {
        let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let (before, after, changes) = commit(&mut host, id, payload);
        let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
            .unwrap_or_else(|| panic!("{label}: a single model insert must be admitted"));
        let mut controller = CadSceneController::default();
        prepare_fonts(&mut controller, before, Some(fonts.clone()), None);
        prepare_fonts(
            &mut controller,
            after.clone(),
            Some(fonts.clone()),
            Some(&receipt),
        );
        let ready = controller.ready.as_ref().unwrap();
        assert!(ready.base_append.is_some(), "{label}: append proof");
        assert!(
            !ready.base.to_delta().added.is_empty(),
            "{label}: the entity draws"
        );
        let full = build_scene_with_fonts(&after, ready.base.stamp().clone(), Some(fonts.clone()))
            .unwrap();
        assert_eq!(ready.base.to_delta().added, full.added, "{label}: batches");
        assert_eq!(ready.base.images(), full.images, "{label}: images");
        assert_eq!(ready.base.masks(), full.masks, "{label}: masks");
    }
}

#[test]
fn point_insert_is_restricted_to_the_full_rebuild() {
    // A visible POINT's glyph is drawing-wide marker overlay state
    // (`$PDMODE`/`$PDSIZE` scan) that only a full base rebuild recomputes, so the
    // receipt must decline even though the insert is otherwise well-formed.
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let (before, after, changes) = commit(
        &mut host,
        CommandId::CreatePoint,
        CommandPayload::Points(vec![p(5.0, 6.0)]),
    );
    assert_eq!(dirty_entities(&changes).len(), 1);
    assert!(DrawingEditReceipt::from_commit(before, after, &changes).is_none());

    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let (_, receipt) = host
        .execute_traced_with_edit(Command {
            schema_version: 1,
            id: CommandId::CreatePoint,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload: CommandPayload::Points(vec![p(7.0, 8.0)]),
        })
        .unwrap();
    assert!(receipt.is_none(), "the host funnel declines a POINT insert");
}

/// Insert one entity through the sanctioned transaction path on a clone of
/// `before`, returning the new snapshot and the changeset the commit published.
fn insert_entity(
    before: &Arc<DrawingDatabase>,
    geometry: SemanticGeometry,
    type_key: &str,
) -> (Arc<DrawingDatabase>, ChangeSet) {
    let mut after = before.as_ref().clone();
    let id = after.allocate_entity_id();
    let draw_order = before
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .unwrap_or(-1)
        + 1;
    let mut transaction = after
        .begin_drawing_transaction("test: single insert", TransactionId(1))
        .unwrap();
    transaction
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(id.0),
                type_key: type_key.into(),
                revision: Revision(0),
                source_handle: None,
            },
            id,
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order,
        })
        .unwrap();
    let changes = transaction.commit().unwrap();
    (Arc::new(after), changes)
}

#[test]
fn receipt_declines_an_image_insert() {
    let host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = host.drawing().unwrap();
    let (after, changes) = insert_entity(
        &before,
        SemanticGeometry::Image {
            origin: p(0.0, 0.0),
            u: p(1.0, 0.0),
            v: p(0.0, 1.0),
            pixels: [1.0, 1.0],
            file: None,
            clip: None,
            visible: true,
        },
        "AcDbRasterImage",
    );
    assert!(changes.follows(before.id(), before.revision()));
    assert!(matches!(
        changes.changes.as_slice(),
        [ObjectChange::Insert(_)]
    ));
    assert!(
        DrawingEditReceipt::from_commit(before, after, &changes).is_none(),
        "an IMAGE insert always contributes raster draw items the append path cannot upload"
    );
}

#[test]
fn receipt_declines_a_mask_insert() {
    let host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let before = host.drawing().unwrap();
    let (after, changes) = insert_entity(
        &before,
        SemanticGeometry::Mask {
            boundary: vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0)],
            inverted: false,
        },
        "AcDbWipeout",
    );
    assert!(changes.follows(before.id(), before.revision()));
    assert!(matches!(
        changes.changes.as_slice(),
        [ObjectChange::Insert(_)]
    ));
    assert!(
        DrawingEditReceipt::from_commit(before, after, &changes).is_none(),
        "a WIPEOUT/MASK insert always contributes mask draw items the append path cannot upload"
    );
}

#[test]
fn appendable_delta_requires_no_image_or_mask_draw_items() {
    let delta = SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 1),
        added: vec![],
        removed_chunks: vec![],
        images: vec![],
        masks: vec![],
    };
    assert!(super::appendable_delta(&delta));

    let with_image = SceneDelta {
        images: vec![ImageBatch {
            resource: ResourceKey("yacr-test.png".into()),
            transform: Transform3::identity(),
            clip: None,
            alpha: 1.0,
            draw_order: 0,
            sources: vec![],
        }],
        ..delta.clone()
    };
    assert!(!super::appendable_delta(&with_image));

    let with_mask = SceneDelta {
        masks: vec![MaskBatch {
            polygon: Arc::from(vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0)]),
            inverted: false,
            draw_order: 0,
            sources: vec![],
        }],
        ..delta
    };
    assert!(!super::appendable_delta(&with_mask));
}

/// A model-space database whose block definition holds one wipeout mask member,
/// so inserting an `INSERT` of that block yields a per-entity delta that carries
/// `SceneDelta::masks` — the append-decline trigger.
fn mask_block_database() -> (Arc<DrawingDatabase>, BlockId) {
    let block = BlockId(1);
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(10.0, 0.0),
            },
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "AcDbWipeout".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Block(block),
            geometry: SemanticGeometry::Mask {
                boundary: vec![p(2.0, 2.0), p(4.0, 2.0), p(4.0, 4.0)],
                inverted: false,
            },
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: block,
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    (Arc::new(builder.finish().unwrap()), block)
}

fn declined_append_fixture() -> (
    Arc<DrawingDatabase>,
    Arc<DrawingDatabase>,
    DrawingEditReceipt,
) {
    let (before, block) = mask_block_database();
    let (after, changes) = insert_entity(
        &before,
        SemanticGeometry::Insert {
            block,
            transform: Transform3::identity(),
        },
        "AcDbBlockReference",
    );
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("an INSERT is append-eligible by class");
    (before, after, receipt)
}

#[test]
fn declined_append_rescans_point_markers_like_a_full_rebuild() {
    let (before, after, receipt) = declined_append_fixture();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    controller.fail_after_point_staging = true;
    let declined = controller.prepare_shared_with_overlays_and_images_and_edit(
        Some(after.clone()),
        DocumentId(1),
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        &OverlayInputs::default(),
        None,
        Some(&receipt),
    );
    assert!(
        declined.is_err(),
        "a declined append is a full rebuild and must rescan the POINT markers"
    );

    // Without the hook the same transition publishes a full rebuild, never an
    // append proof, and the CPU scene equals a normal full build.
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_none(), "the append was declined");
    let full = build_scene(&after, ready.base.stamp().clone()).unwrap();
    assert!(
        !full.masks.is_empty(),
        "the block's wipeout member is what declined the append"
    );
    assert_eq!(ready.base.to_delta().added, full.added);
    assert_eq!(ready.base.images(), full.images);
    assert_eq!(ready.base.masks(), full.masks);
}

#[test]
fn taken_append_skips_the_point_marker_rescan() {
    let (before, after, changes) = transition();
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes).unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    controller.fail_after_point_staging = true;
    let taken = controller.prepare_shared_with_overlays_and_images_and_edit(
        Some(after),
        DocumentId(1),
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
        &OverlayInputs::default(),
        None,
        Some(&receipt),
    );
    assert!(
        taken.is_ok(),
        "a taken append must not run the drawing-wide POINT marker scan"
    );
    assert!(controller.ready.as_ref().unwrap().base_append.is_some());
}

#[test]
fn receipt_requires_one_insert_and_the_published_revision_pair() {
    let (before, after, published) = transition();
    let changes = published.as_ref();
    let inserted = match changes.changes.as_slice() {
        [ObjectChange::Insert(object)] => *object,
        other => panic!("a LINE commit inserts exactly one object, got {other:?}"),
    };
    let inserted_entity = EntityId(inserted.0);

    // A revision-only publish (the layer/point-style shape) has no entries.
    let empty = ChangeSet {
        changes: Vec::new(),
        ..changes.clone()
    };
    assert!(DrawingEditReceipt::from_commit(before.clone(), after.clone(), &empty).is_none());

    // The very same Arc is not a transition.
    assert!(DrawingEditReceipt::from_commit(before.clone(), before.clone(), changes).is_none());
    // A different Arc with identical content is not the published snapshot.
    let cloned = Arc::new(before.as_ref().clone());
    assert!(DrawingEditReceipt::from_commit(before.clone(), cloned, changes).is_none());
    // Reversed snapshots: the changeset follows `before`, not `after`.
    assert!(DrawingEditReceipt::from_commit(after.clone(), before.clone(), changes).is_none());
    // Wrong `before` revision.
    let wrong_before = ChangeSet {
        before: Revision(changes.before.0 + 1),
        ..changes.clone()
    };
    assert!(
        DrawingEditReceipt::from_commit(before.clone(), after.clone(), &wrong_before).is_none()
    );

    // Two distinct inserted ids in one commit.
    let two_inserts = ChangeSet {
        changes: vec![
            ObjectChange::Insert(ObjectId(9001)),
            ObjectChange::Insert(ObjectId(9002)),
        ],
        ..changes.clone()
    };
    assert_eq!(dirty_entities(&two_inserts).len(), 2);
    assert!(DrawingEditReceipt::from_commit(before.clone(), after.clone(), &two_inserts).is_none());

    // Update-only: the single dirty id was never inserted.
    let update_only = ChangeSet {
        changes: vec![ObjectChange::Update(inserted, ChangeMask::STYLE)],
        ..changes.clone()
    };
    assert_eq!(
        dirty_entities(&update_only),
        vec![(inserted_entity, ChangeMask::STYLE)]
    );
    assert!(DrawingEditReceipt::from_commit(before.clone(), after.clone(), &update_only).is_none());

    // The reported id already exists before the commit.
    let already_present = ChangeSet {
        changes: vec![ObjectChange::Insert(ObjectId(1))],
        ..changes.clone()
    };
    assert!(
        DrawingEditReceipt::from_commit(before.clone(), after.clone(), &already_present).is_none()
    );

    // The documented insert-with-display shape: `Insert(id)` followed by
    // `Update(id, STYLE)` for the *same* id is one dirty entity and is admitted.
    let insert_with_style = ChangeSet {
        changes: vec![
            ObjectChange::Insert(inserted),
            ObjectChange::Update(inserted, ChangeMask::STYLE),
        ],
        ..changes.clone()
    };
    assert_eq!(
        dirty_entities(&insert_with_style),
        vec![(inserted_entity, ChangeMask::STYLE)]
    );
    assert!(
        DrawingEditReceipt::from_commit(before.clone(), after.clone(), &insert_with_style)
            .is_some()
    );

    // A plain insert is one dirty id; `Insert` carries no mask of its own.
    assert_eq!(
        dirty_entities(changes),
        vec![(inserted_entity, ChangeMask(0))]
    );
}

fn issued_edit(before: Arc<DrawingDatabase>) -> (Arc<DrawingDatabase>, DrawingEditReceipt) {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    host.application
        .workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = before;
    let (trace, receipt) = host
        .execute_traced_with_edit(Command {
            schema_version: 1,
            id: CommandId::CreateLine,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload: line_points(),
        })
        .unwrap();
    trace.result.unwrap();
    (host.drawing().unwrap(), receipt.unwrap())
}

#[test]
fn published_base_not_last_adopted_document_authorizes_append() {
    let (before, after, changes) = transition();
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes).unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    // A borrowed preparation replaces the base without adopting a shared Arc.
    controller
        .prepare(
            Some(&after),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_none());
    assert_eq!(
        ready.base.to_delta().added,
        build_scene(&after, ready.base.stamp().clone())
            .unwrap()
            .added
    );

    let (next, receipt) = issued_edit(after.clone());
    let mut controller = CadSceneController::default();
    prepare(&mut controller, receipt.before.clone(), None);
    // Adoption of B fails: the published base still belongs to A.
    assert!(controller
        .prepare_shared(
            Some(next.clone()),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(cad_domain::LayoutId(99999))
        )
        .is_err());
    // Use a different B -> C receipt: A must not be treated as B's base.
    let (third, third_receipt) = issued_edit(next);
    prepare(&mut controller, third.clone(), Some(&third_receipt));
    assert!(controller.ready.as_ref().unwrap().base_append.is_none());
    assert_eq!(
        controller.ready.as_ref().unwrap().base.to_delta().added,
        build_scene(
            &third,
            controller.ready.as_ref().unwrap().base.stamp().clone()
        )
        .unwrap()
        .added
    );
}

#[test]
fn sequential_appends_bind_each_immediate_predecessor() {
    let (before, _, _) = transition();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before.clone(), None);
    let mut previous = controller.ready.as_ref().unwrap().base.clone();
    let mut drawing = before;
    for _ in 0..2 {
        let (after, receipt) = issued_edit(drawing);
        assert!(Arc::ptr_eq(receipt.after(), &after));
        prepare(&mut controller, after.clone(), Some(&receipt));
        let ready = controller.ready.as_ref().unwrap();
        assert!(ready
            .base_append
            .as_ref()
            .unwrap()
            .predecessor
            .same_as(&previous));
        assert_eq!(
            ready.base.to_delta().added,
            build_scene(&after, ready.base.stamp().clone())
                .unwrap()
                .added
        );
        assert_eq!(ready.base.stamp().object_revision, after.revision());
        previous = ready.base.clone();
        drawing = after;
    }
}

#[test]
fn hidden_line_appends_zero_batches_and_resource_changes_fall_back() {
    let (before, after, changes) = transition();
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes).unwrap();
    let inserted = match &receipt.delta {
        super::edit::CommitDelta::Append(inserted) => *inserted,
        other => panic!("expected an append receipt, got {other:?}"),
    };
    let layer = after.entity(inserted).unwrap().layer;
    let mut hidden = LayerOverrideSet::new();
    hidden.set(layer, false);
    let mut controller = CadSceneController::default();
    controller
        .prepare_shared(
            Some(before.clone()),
            DocumentId(1),
            None,
            &hidden,
            SpaceSelection::Model,
        )
        .unwrap();
    let count = controller.ready.as_ref().unwrap().base.batch_len();
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(after.clone()),
            DocumentId(1),
            None,
            &hidden,
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            Some(&receipt),
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_some());
    assert_eq!(ready.base.to_delta().added.len(), count);
    assert_eq!(
        ready.base.to_delta().added,
        build_scene_with_overrides(&after, ready.base.stamp().clone(), None, &hidden)
            .unwrap()
            .added
    );

    for change_font in [false, true] {
        let mut controller = CadSceneController::default();
        controller
            .prepare_shared(
                Some(before.clone()),
                DocumentId(1),
                Some(Arc::new(FontEngine::new())),
                &hidden,
                SpaceSelection::Model,
            )
            .unwrap();
        let fonts = if change_font {
            Some(Arc::new(FontEngine::new()))
        } else {
            None
        };
        controller
            .prepare_shared_with_overlays_and_images_and_edit(
                Some(after.clone()),
                DocumentId(1),
                fonts,
                &hidden,
                SpaceSelection::Model,
                &OverlayInputs::default(),
                None,
                Some(&receipt),
            )
            .unwrap();
        assert!(controller.ready.as_ref().unwrap().base_append.is_none());
    }
}

#[test]
fn failed_full_build_does_not_poison_points_for_subsequent_valid_append() {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    host.create_point(p(5.0, 6.0)).unwrap();
    let before = host.drawing().unwrap();
    host.create_point(p(90.0, 60.0)).unwrap();
    let rejected = host.drawing().unwrap();
    let (after, receipt) = issued_edit(before.clone());
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    let points = controller.point_markers.clone();
    let style = controller.point_display_style;
    let base = controller.ready.as_ref().unwrap().base.clone();
    controller.fail_after_point_staging = true;
    assert!(controller
        .prepare_shared(
            Some(rejected),
            DocumentId(1),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model
        )
        .is_err());
    assert_eq!(controller.point_markers, points);
    assert_eq!(controller.point_display_style, style);
    assert!(controller.ready.as_ref().unwrap().base.same_as(&base));
    prepare(&mut controller, after.clone(), Some(&receipt));
    assert!(controller.ready.as_ref().unwrap().base_append.is_some());
    let mut full = CadSceneController::default();
    prepare(&mut full, after, None);
    assert_eq!(controller.point_markers, full.point_markers);
    assert_eq!(controller.point_display_style, full.point_display_style);
    assert_eq!(
        controller.ready.as_ref().unwrap().highlight.added,
        full.ready.as_ref().unwrap().highlight.added
    );
}

#[test]
fn styled_dashed_line_uses_the_full_provider_and_compaction_path() {
    use cad_db::{DrawingDatabaseBuilder, Layer, LayerAttributes, LineType, LinetypePattern};
    use cad_domain::{DatabaseId, LayerId, LinetypeId};
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_linetype(LineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.25, -0.125]),
            complex: false,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(0),
            LayerAttributes {
                linetype: "DASHED".into(),
                opacity: 0.5,
                ..LayerAttributes::default()
            },
        )
        .unwrap();
    let before = Arc::new(builder.finish().unwrap());
    let (after, receipt) = issued_edit(before.clone());
    let mut controller = CadSceneController::default();
    prepare(&mut controller, before, None);
    prepare(&mut controller, after.clone(), Some(&receipt));
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.base_append.is_some());
    assert!(!ready.base.to_delta().added.is_empty());
    assert!(ready
        .base
        .to_delta()
        .added
        .iter()
        .all(|batch| batch.alpha == 0.5));
    assert!(ready
        .base
        .batches()
        .all(|batch| !batch.linetype.elements.is_empty()));
    assert_eq!(
        ready.base.to_delta().added,
        build_scene(&after, ready.base.stamp().clone())
            .unwrap()
            .added
    );
}

#[test]
fn changed_images_layers_and_vendor_snapshot_identity_force_full_build() {
    let (before, after, changes) = transition();
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes).unwrap();
    for mode in 0..3 {
        let mut controller = CadSceneController::default();
        controller.set_vendor_representations(Some(Arc::new(VendorRepresentationStore::empty(7))));
        let images = Arc::new(DecodedImageCache::new());
        let layers = LayerOverrideSet::new();
        controller
            .prepare_shared_with_overlays_and_images(
                Some(before.clone()),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                &OverlayInputs::default(),
                Some(images.clone()),
            )
            .unwrap();
        let mut layers = layers;
        let images = if mode == 0 {
            Arc::new(DecodedImageCache::new())
        } else {
            images
        };
        if mode == 1 {
            layers.set(cad_domain::LayerId(0), false);
        }
        if mode == 2 {
            controller
                .set_vendor_representations(Some(Arc::new(VendorRepresentationStore::empty(7))));
        }
        controller
            .prepare_shared_with_overlays_and_images_and_edit(
                Some(after.clone()),
                DocumentId(1),
                None,
                &layers,
                SpaceSelection::Model,
                &OverlayInputs::default(),
                Some(images),
                Some(&receipt),
            )
            .unwrap();
        assert!(
            controller.ready.as_ref().unwrap().base_append.is_none(),
            "mode={mode}"
        );
    }
}
