//! Draw-order base for the shared create funnel (`insert_new_entities`).
//!
//! The funnel takes one past the highest **model-space** `draw_order` in a
//! single pass over the stored rows (it used to allocate and sort
//! `model_space()` just to take the maximum). These contracts pin the resulting
//! order and that the filter did not drift to another space.

use super::*;

/// Model rows stored in id order but carrying **unsorted** draw orders, plus
/// paper-space and block-definition rows whose draw orders (900/700) are far
/// above the model maximum (11). Used to pin the create funnel's draw-order base.
fn app_with_unsorted_draw_orders() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(0),
        entities: vec![EntityId(5)],
        dynamic_visibility: None,
    })
    .unwrap();
    let line = || SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(1.0, 0.0),
    };
    b.insert_entity(ent(1, line(), 7)).unwrap();
    b.insert_entity(ent(2, line(), 2)).unwrap();
    b.insert_entity(ent(3, line(), 11)).unwrap();
    let mut paper = ent(4, line(), 900);
    paper.space = SpaceId::Paper(LayoutId(1));
    b.insert_entity(paper).unwrap();
    let mut child = ent(5, line(), 700);
    child.space = SpaceId::Block(cad_domain::BlockId(0));
    b.insert_entity(child).unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

#[test]
fn create_line_draw_order_ignores_paper_block_rows_and_id_order() {
    let (mut app, mut session) = app_with_unsorted_draw_orders();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap();
    let created = EntityId(outcome.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let entity = drawing.entity(created).expect("the created line");
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(
        entity.draw_order, 12,
        "one past the model maximum (11); the paper (900) and block (700) rows do not count"
    );
    assert_eq!(
        drawing.entity(EntityId(4)).unwrap().draw_order,
        900,
        "the paper row is untouched"
    );
    assert_eq!(
        drawing.entity(EntityId(5)).unwrap().draw_order,
        700,
        "the block row is untouched"
    );
}

#[test]
fn offset_a_paper_space_entity_keeps_the_model_draw_order_base() {
    let (mut app, mut session) = app_with_unsorted_draw_orders();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(4)],
            side_point: point(0.5, 1.0),
        },
    )
    .unwrap();
    let created = EntityId(outcome.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let entity = drawing.entity(created).expect("the offset line");
    assert_eq!(
        entity.space,
        SpaceId::Paper(LayoutId(1)),
        "OFFSET keeps the source space"
    );
    assert_eq!(
        entity.draw_order, 12,
        "the create funnel keeps the model-space base (11 + 1), not the target space's 900"
    );
}
