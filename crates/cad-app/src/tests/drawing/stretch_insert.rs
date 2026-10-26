//! STRETCH contract tests for `INSERT` (block reference) partial enclosures.
//!
//! Pins the declared rule documented in `crate::app_drawing::stretch` and
//! `docs/compatibility.md`:
//!
//! * a **partially** enclosed block reference is translated **rigidly** by the
//!   drag delta **iff** its insertion point (the transformed block-space origin)
//!   lies inside the crossing window — the linear part of the placement
//!   (rotation / scale / mirror columns) and the block id stay **bit-identical**,
//!   so the reference is never re-fitted;
//! * otherwise the reference is left **bit-identically** unchanged and the reason
//!   is named (`drawing.stretch.insert_outside`), with nothing staged;
//! * a **fully** enclosed reference keeps the ordinary whole-move path, and a
//!   reference outside the window is not touched at all.
//!
//! Every moving case asserts the transaction/undo shape (one transaction for the
//! whole window, one `update_entity` per changed entity, one undo entry, undo
//! restoring the stored placement bit-exactly); every non-moving case asserts
//! that **nothing** was written (revision unchanged, undo depth 0, geometry
//! bit-identical).

use super::*;

use cad_db::BlockDefinition;

/// Build a drawing with layer 0 plus the given blocks and entities.
fn build(entities: Vec<DbEntity>, blocks: Vec<BlockDefinition>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// A model-space or block-space entity.
fn ent_in(id: u128, space: SpaceId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: order,
    }
}

/// The two opposite corners of a crossing window, as the **canvas logical**
/// corners the payload expects, from the two world corners a test thinks in.
fn window(a: Point3, b: Point3) -> [[f64; 2]; 2] {
    let vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    let size = vp.logical_size;
    let to_logical = |world: Point3| {
        vp.camera
            .world_to_screen(world, size)
            .expect("finite world point")
    };
    [to_logical(a), to_logical(b)]
}

fn stretch(
    app: &mut Application,
    session: &mut SessionState,
    rect: [[f64; 2]; 2],
    base: Point3,
    delta: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: rect,
            base,
            delta,
        },
    )
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry_of(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap()
        .geometry
        .clone()
}

fn has_code(outcome: &CommandOutcome, code: &str) -> bool {
    outcome.diagnostics.iter().any(|d| d.code == code)
}

fn diagnostic_message(outcome: &CommandOutcome, code: &str) -> Option<String> {
    outcome
        .diagnostics
        .iter()
        .find(|d| d.code == code)
        .map(|d| d.message.clone())
}

/// The stored `(block, transform)` of the INSERT `id`.
fn insert_of(app: &Application, id: u128) -> (BlockId, Transform3) {
    match geometry_of(app, id) {
        SemanticGeometry::Insert { block, transform } => (block, transform),
        other => panic!("expected an insert, got {other:?}"),
    }
}

/// The placement's translation column — the insertion point, read straight out of
/// the stored matrix so the assertion is about stored bits, not a re-derivation.
fn translation_column(transform: &Transform3) -> Point3 {
    Point3 {
        x: transform.matrix[0][3],
        y: transform.matrix[1][3],
        z: transform.matrix[2][3],
    }
}

/// The 3x4 linear part (plus the projection row) of the placement: what a rigid
/// move must copy bit-identically.
fn linear_part(transform: &Transform3) -> [[f64; 3]; 4] {
    let m = transform.matrix;
    [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
        [m[3][0], m[3][1], m[3][2]],
    ]
}

/// A drawing with block 0 (one member LINE `(0,0)-(1,0)`) and one INSERT at
/// `(5, 5)` with the given placement. The displayed member spans `(5,5)-(6,5)`.
fn insert_app(placement: Transform3) -> (Application, SessionState) {
    let member = ent_in(
        100,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    );
    let insert = ent(
        1,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: placement,
        },
        1,
    );
    build(
        vec![member, insert],
        vec![BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    )
}

fn plain_insert_app() -> (Application, SessionState) {
    insert_app(Transform3::translation(p3(5.0, 5.0, 0.0)))
}

#[test]
fn stretch_insert_moves_rigidly_when_the_insertion_point_is_inside_the_window() {
    let (mut app, mut session) = plain_insert_app();
    let (block_before, transform_before) = insert_of(&app, 1);
    let member_before = geometry_of(&app, 100);
    let geometry_before = geometry_of(&app, 1);
    let before = revision(&app);

    // Window `(4.5,4.5)-(5.75,5.75)` holds the insertion point `(5,5)` but not the
    // member's far end `(6,5)`, so the reference is **partially** enclosed.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(4.5, 4.5), point(5.75, 5.75)),
        point(0.0, 0.0),
        point(2.0, 3.0),
    )
    .unwrap();

    let (block_after, transform_after) = insert_of(&app, 1);
    assert_eq!(
        block_after, block_before,
        "the block reference is unchanged"
    );
    assert_eq!(
        translation_column(&transform_after),
        point(7.0, 8.0),
        "the insertion point lands on old + delta"
    );
    assert_eq!(
        linear_part(&transform_after),
        linear_part(&transform_before),
        "a rigid move copies the linear part bit-identically"
    );
    // The whole reference is one rigid translation: every point of the displayed
    // member is old + delta, and the block definition is untouched.
    assert_eq!(
        transform_after.apply_point(point(0.0, 0.0)),
        point(7.0, 8.0)
    );
    assert_eq!(
        transform_after.apply_point(point(1.0, 0.0)),
        point(8.0, 8.0)
    );
    assert_eq!(
        geometry_of(&app, 100),
        member_before,
        "block members are never rewritten by this path"
    );

    // One transaction, one update, one undo entry. STRETCH stages every kind with
    // `update_entity`, so the mask is GEOMETRY (not the MOVE-only TRANSFORM
    // fast-path mask) — the same shape a per-vertex stretch has.
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    let changes = outcome.changes.as_ref().expect("one committed transaction");
    assert_eq!(
        changes.changes,
        vec![cad_db::ObjectChange::Update(
            ObjectId(1),
            cad_db::ChangeMask::GEOMETRY
        )],
        "the reference moves by a single update_entity"
    );
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry");
    assert!(
        !has_code(&outcome, "drawing.stretch.insert_outside"),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        !has_code(&outcome, "drawing.stretch.not_stretchable"),
        "{:?}",
        outcome.diagnostics
    );

    // One undo restores the stored placement bit-exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, 1), geometry_before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn stretch_insert_with_the_insertion_point_outside_the_window_is_unchanged_and_named() {
    let (mut app, mut session) = plain_insert_app();
    let geometry_before = geometry_of(&app, 1);
    let member_before = geometry_of(&app, 100);
    let before = revision(&app);

    // The window overlaps the displayed member (`x` in `[5.5, 6.5]` meets the
    // member's `(5,5)-(6,5)`) but **not** the insertion point `(5,5)`.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(5.5, 4.5), point(6.5, 5.5)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();

    assert!(
        outcome.changes.is_none(),
        "no transaction when nothing changes"
    );
    assert!(outcome.objects.is_empty(), "no entity changed");
    assert_eq!(revision(&app), before, "the revision is untouched");
    assert_eq!(undo_depth(&app), 0, "no history is recorded");
    assert_eq!(
        geometry_of(&app, 1),
        geometry_before,
        "the placement is bit-identical"
    );
    assert_eq!(
        geometry_of(&app, 100),
        member_before,
        "the block member is bit-identical"
    );

    let message =
        diagnostic_message(&outcome, "drawing.stretch.insert_outside").unwrap_or_else(|| {
            panic!(
                "expected the named insert diagnostic: {:?}",
                outcome.diagnostics
            )
        });
    assert!(
        message.contains("insertion point"),
        "the message names the precondition: {message}"
    );
    // The named reason replaces the generic move-only code for this kind: an
    // INSERT is not in the move-only list any more.
    assert!(
        !has_code(&outcome, "drawing.stretch.not_stretchable"),
        "{:?}",
        outcome.diagnostics
    );
    // The whole command still reports that nothing moved.
    assert!(
        has_code(&outcome, "drawing.stretch"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_fully_enclosed_insert_still_moves_whole() {
    let (mut app, mut session) = plain_insert_app();
    let (_, transform_before) = insert_of(&app, 1);
    let before = revision(&app);

    // The window contains the whole displayed member and its bounds, so the
    // enclosure is Full and the ordinary whole-move path applies.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(4.0, 4.0), point(7.0, 7.0)),
        point(0.0, 0.0),
        point(2.0, 3.0),
    )
    .unwrap();

    let (block_after, transform_after) = insert_of(&app, 1);
    assert_eq!(block_after, BlockId(0));
    assert_eq!(translation_column(&transform_after), point(7.0, 8.0));
    assert_eq!(
        linear_part(&transform_after),
        linear_part(&transform_before)
    );
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry");
    assert!(
        !has_code(&outcome, "drawing.stretch.insert_outside"),
        "{:?}",
        outcome.diagnostics
    );
}

/// A placement with a **30° rotation and a non-uniform scale** (×2 along the block
/// X axis, ×0.5 along the block Y axis): the displayed member runs from `(5,5)` to
/// `(5 + 2·cos30°, 5 + 2·sin30°)`.
fn rotated_scaled_placement() -> Transform3 {
    let angle = std::f64::consts::FRAC_PI_6;
    let (sin, cos) = angle.sin_cos();
    Transform3::from_basis(
        p3(2.0 * cos, 2.0 * sin, 0.0),
        p3(-0.5 * sin, 0.5 * cos, 0.0),
        p3(5.0, 5.0, 0.0),
    )
}

#[test]
fn stretch_rotated_scaled_insert_moves_rigidly_and_is_not_refitted() {
    let placement = rotated_scaled_placement();
    let (mut app, mut session) = insert_app(placement);
    let (_, transform_before) = insert_of(&app, 1);
    let geometry_before = geometry_of(&app, 1);
    let before = revision(&app);

    // The insertion point `(5,5)` is inside; the member's far end
    // `(6.7320…, 6.0)` is not, so the enclosure is Partial.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(4.5, 4.5), point(6.0, 5.5)),
        point(0.0, 0.0),
        point(1.0, -2.0),
    )
    .unwrap();

    let (_, transform_after) = insert_of(&app, 1);
    assert_eq!(
        linear_part(&transform_after),
        linear_part(&transform_before),
        "no re-fit: the rotation/scale columns are copied bit-identically"
    );
    let translated = translation_column(&transform_after);
    assert_eq!(
        translated.x,
        5.0 + 1.0,
        "insertion point + delta, bit for bit"
    );
    assert_eq!(
        translated.y,
        5.0 - 2.0,
        "insertion point + delta, bit for bit"
    );
    assert_eq!(translated.z, 0.0);
    // The reference is still a rigid body: the same offset from the insertion
    // point as before the move.
    assert_eq!(
        cad_geometry::sub(transform_after.apply_point(point(1.0, 0.0)), translated),
        cad_geometry::sub(
            transform_before.apply_point(point(1.0, 0.0)),
            translation_column(&transform_before)
        ),
        "the member keeps its placement relative to the insertion point"
    );
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, 1), geometry_before, "undo is bit-exact");
}

#[test]
fn stretch_insert_and_line_in_one_window_share_one_transaction_and_one_undo() {
    let member = ent_in(
        100,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    );
    let insert = ent(
        1,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        1,
    );
    let line = ent(
        2,
        SemanticGeometry::Line {
            start: point(5.0, 5.2),
            end: point(9.0, 5.2),
        },
        2,
    );
    let (mut app, mut session) = build(
        vec![member, insert, line],
        vec![BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    );
    let insert_before = geometry_of(&app, 1);
    let line_before = geometry_of(&app, 2);
    let before = revision(&app);

    // The same window: the INSERT is partially enclosed with its insertion point
    // inside (rigid move), the LINE has its start `(5,5.2)` inside (per-vertex
    // stretch), and both are handled in the one plan.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(4.5, 4.5), point(5.75, 5.75)),
        point(0.0, 0.0),
        point(0.0, 2.0),
    )
    .unwrap();

    assert_eq!(
        translation_column(&insert_of(&app, 1).1),
        point(5.0, 7.0),
        "the block reference moves by the delta"
    );
    match geometry_of(&app, 2) {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(start, point(5.0, 7.2), "the in-window line start moved");
            assert_eq!(end, point(9.0, 5.2), "the out-of-window end is put");
        }
        other => panic!("expected a line, got {other:?}"),
    }

    let changes = outcome.changes.as_ref().expect("one committed transaction");
    let updated: Vec<u128> = changes
        .changes
        .iter()
        .map(|change| match change {
            // STRETCH stages with `update_entity` for every kind, so both the rigid
            // INSERT move and the per-vertex LINE stretch carry GEOMETRY.
            cad_db::ObjectChange::Update(object, mask) => {
                assert_eq!(*mask, cad_db::ChangeMask::GEOMETRY);
                object.0
            }
            other => panic!("expected an update, got {other:?}"),
        })
        .collect();
    assert_eq!(updated.len(), 2, "two updates in one transaction");
    assert!(updated.contains(&1) && updated.contains(&2), "{updated:?}");
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry for both");

    // One undo restores both entities bit-exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, 1), insert_before);
    assert_eq!(geometry_of(&app, 2), line_before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn stretch_command_moves_a_partially_enclosed_insert() {
    let (mut app, mut session) = plain_insert_app();
    let before = revision(&app);

    // Command level: build the `Command` exactly as the host does (id + document +
    // viewport + payload) and drive `Application::execute`.
    let cmd = command(
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: window(point(4.5, 4.5), point(5.75, 5.75)),
            base: point(10.0, 10.0),
            delta: point(12.5, 14.0),
        },
    );
    assert_eq!(cmd.id, CommandId::StretchEntities);
    assert_eq!(cmd.document, DocumentId(1));
    assert_eq!(cmd.viewport, ViewportId(1));
    let outcome = app.execute(&mut session, cmd).unwrap();

    // The effective translation is `delta - base`, so the insertion point lands on
    // `(5,5) + (2.5,4)`.
    assert_eq!(translation_column(&insert_of(&app, 1).1), point(7.5, 9.0));
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert!(outcome.changes.is_some(), "one committed transaction");
    assert!(
        outcome.measurement.is_none(),
        "STRETCH reports no measurement"
    );
    let summary = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.stretch")
        .unwrap_or_else(|| panic!("expected the summary diagnostic: {:?}", outcome.diagnostics));
    assert!(
        summary.message.contains("1 entity"),
        "the summary counts the one changed entity: {}",
        summary.message
    );
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry");
}
