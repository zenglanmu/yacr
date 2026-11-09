//! `LIST`, `STATUS` and `FS` contract tests (read-only inquiry commands).
//!
//! Pinned here: LIST's per-entity report (including space and display
//! properties), STATUS's fixture counts and honest `not tracked` lines, FS's
//! direct-touch union, the Viewer-mode admission of all three, and the fact that
//! none of them writes the database, bumps a revision or records undo.

use super::*;
use crate::selection::model_ref;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer, LayerAttributes, Layout,
    LineType, LinetypePattern, Style,
};

fn entity(
    id: u128,
    geometry: SemanticGeometry,
    layer: LayerId,
    space: SpaceId,
    type_key: &str,
    handle: Option<&str>,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: handle.map(str::to_string),
        },
        id: EntityId(id),
        layer,
        space,
        geometry,
        draw_order: id as i64,
    }
}

fn line(id: u128, start: Point3, end: Point3) -> DbEntity {
    entity(
        id,
        SemanticGeometry::Line { start, end },
        LayerId(0),
        SpaceId::Model,
        "AcDbLine",
        None,
    )
}

fn app_from_builder(builder: DrawingDatabaseBuilder) -> (Application, SessionState, DocumentId) {
    let mut app = Application::new();
    let document_id = DocumentId(1);
    let drawing = builder.finish().unwrap();
    app.workspace.documents.insert(
        document_id,
        Document {
            id: document_id,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), document_id, [800.0, 600.0]),
    );
    let session = SessionState::new(document_id, AppMode::Work);
    (app, session, document_id)
}

fn command(id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: 1,
        id,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload,
    }
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn diagnostic<'a>(outcome: &'a CommandOutcome, code: &str) -> Vec<&'a str> {
    outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}

fn list_fixture() -> (Application, SessionState, DocumentId) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name, visible) in [(LayerId(0), "0", true), (LayerId(3), "WALLS", true)] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(7),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(4.0, 3.0),
            },
            LayerId(3),
            SpaceId::Model,
            "AcDbLine",
            Some("1A"),
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Circle {
                center: point(1.0, 2.0),
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: 2.5,
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbCircle",
            None,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            3,
            SemanticGeometry::Text {
                text: "Hello from LIST".into(),
                position: point(0.0, 0.0),
                style: StyleId(0),
                height: 2.5,
                rotation: 0.0,
                font: None,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
                wrap_width: None,
            },
            LayerId(3),
            SpaceId::Model,
            "AcDbText",
            None,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            4,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
            LayerId(0),
            SpaceId::Paper(LayoutId(7)),
            "AcDbLine",
            None,
        ))
        .unwrap();
    app_from_builder(builder)
}

#[test]
fn list_reports_every_selected_object_with_space_and_revision() {
    let (mut app, mut session, document) = list_fixture();
    let mut refs = vec![
        model_ref(document, EntityId(1)),
        model_ref(document, EntityId(2)),
        model_ref(document, EntityId(3)),
        model_ref(document, EntityId(4)),
    ];
    // A block-instance member is not directly addressable: LIST reports it and
    // continues with the remaining objects instead of aborting the whole command.
    refs.push(SelectionRef {
        document,
        entity: EntityId(5),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    });
    session.selection = SelectionSet::from_refs(refs);
    let before = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();

    let outcome = app
        .execute(&mut session, command(CommandId::List, CommandPayload::None))
        .expect("LIST is admitted and reports");

    let objects = diagnostic(&outcome, "list.object");
    assert_eq!(objects.len(), 4, "one report per addressable object");
    assert!(objects.iter().any(|m| m.contains("line start=")));
    assert!(objects.iter().any(|m| m.contains("circle center=")));
    assert!(objects
        .iter()
        .any(|m| m.contains("text \"Hello from LIST\"")));
    assert!(objects.iter().any(|m| m.contains("layer=WALLS")));
    assert!(objects.iter().any(|m| m.contains("space=Model")));
    assert!(
        objects.iter().any(|m| m.contains("space=Paper(7)")),
        "paper space is named with its layout id"
    );
    assert!(objects.iter().any(|m| m.contains("handle=1A")));
    assert!(
        objects
            .iter()
            .any(|m| m.contains("thickness=not-modelled extrusion=not-modelled")),
        "LIST states that thickness/extrusion are not modelled"
    );

    let not_addressable = diagnostic(&outcome, "list.not_addressable");
    assert_eq!(not_addressable.len(), 1);
    assert!(not_addressable[0].contains("not directly addressable"));

    let summary = diagnostic(&outcome, "list.summary");
    assert_eq!(summary.len(), 1);
    assert!(summary[0].contains("5 selected object(s)"));
    assert!(summary[0].contains(&format!("drawing revision {}", before.revision().0)));

    // Read-only: no revision bump, no undo entry.
    let after = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();
    assert_eq!(after.revision(), before.revision());
    assert!(!app.can_undo(&document));
}

#[test]
fn list_includes_canonical_non_bylayer_display_properties() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(1, point(0.0, 0.0), point(1.0, 1.0)))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            cad_db::EntityRenderAttributes {
                color: cad_db::EntityColor::Explicit([255, 0, 0]),
                linetype: cad_db::EntityLineType::ByLayer,
                lineweight: cad_db::EntityLineWeight::Explicit(0.35),
                transparency: cad_db::EntityTransparency::Explicit(0.5),
                ..Default::default()
            },
        )
        .unwrap();
    let (mut app, mut session, document) = app_from_builder(builder);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);

    let outcome = app
        .execute(&mut session, command(CommandId::List, CommandPayload::None))
        .expect("LIST reports the entity");
    let message = diagnostic(&outcome, "list.object")[0];
    assert!(message.contains("color=aci:1"), "{message}");
    assert!(message.contains("linetype=ByLayer"), "{message}");
    assert!(message.contains("lineweight=0.35"), "{message}");
    assert!(message.contains("transparency=50"), "{message}");
}

#[test]
fn list_reports_paper_and_block_space_with_every_modelled_display_property() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(7),
            name: "Sheet".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(4),
            name: "ANNO".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(9),
            entities: vec![EntityId(11)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(9), "DOOR", point(0.0, 0.0))
        .unwrap();

    // Paper-space line with a non-default linetype scale and an explicit
    // transparency.
    builder
        .insert_entity(entity(
            4,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
            LayerId(0),
            SpaceId::Paper(LayoutId(7)),
            "AcDbLine",
            None,
        ))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(4),
            cad_db::EntityRenderAttributes {
                linetype_scale: 2.5,
                transparency: cad_db::EntityTransparency::Explicit(0.25),
                ..Default::default()
            },
        )
        .unwrap();

    // A block-definition member, selected directly (not through an INSERT).
    builder
        .insert_entity(entity(
            11,
            SemanticGeometry::Text {
                text: "member".into(),
                position: point(0.0, 0.0),
                style: StyleId(4),
                height: 1.0,
                rotation: 0.0,
                font: None,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
                wrap_width: None,
            },
            LayerId(0),
            SpaceId::Block(BlockId(9)),
            "AcDbText",
            None,
        ))
        .unwrap();

    // INSERT naming the block.
    builder
        .insert_entity(entity(
            12,
            SemanticGeometry::Insert {
                block: BlockId(9),
                transform: Transform3::identity(),
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbBlockReference",
            None,
        ))
        .unwrap();

    // DIMENSION carrying its style-name snapshot.
    let dim_style = cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".into(),
        StyleId(0),
        None,
    );
    builder
        .insert_entity(entity(
            13,
            SemanticGeometry::Dimension(Box::new(
                DimensionGeometry::synthesized(
                    DimensionDefinition {
                        kind: DimensionKind::Aligned,
                        points: DimensionPoints::Linear {
                            first: point(0.0, 0.0),
                            second: point(10.0, 0.0),
                            dimension_line: point(5.0, -4.0),
                        },
                        text_middle: point(0.0, 0.0),
                        text_rotation: 0.0,
                        normal: Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 1.0,
                        },
                    },
                    "Standard".into(),
                    dim_style,
                    None,
                )
                .unwrap(),
            )),
            LayerId(0),
            SpaceId::Model,
            "AcDbDimension",
            Some("D1"),
        ))
        .unwrap();

    // A source-hidden entity reports visible=false.
    builder
        .insert_entity(entity(
            14,
            SemanticGeometry::Line {
                start: point(2.0, 2.0),
                end: point(3.0, 3.0),
            },
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            None,
        ))
        .unwrap();
    builder.set_hidden_entities([EntityId(14)]).unwrap();

    let (mut app, mut session, document) = app_from_builder(builder);
    session.selection = SelectionSet::from_refs(
        [4u128, 11, 12, 13, 14].map(|id| model_ref(document, EntityId(id))),
    );
    let before = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();
    let outcome = app
        .execute(&mut session, command(CommandId::List, CommandPayload::None))
        .expect("LIST reads every directly addressable space");

    let objects = diagnostic(&outcome, "list.object");
    assert_eq!(objects.len(), 5, "one row per addressable entity");
    let paper = objects
        .iter()
        .find(|message| message.contains("id=4 "))
        .expect("the paper-space row");
    assert!(paper.contains("space=Paper(7)"), "{paper}");
    assert!(paper.contains("ltscale=2.500000"), "{paper}");
    assert!(paper.contains("transparency=75"), "{paper}");
    assert!(paper.contains("visible=true"), "{paper}");
    let block = objects
        .iter()
        .find(|message| message.contains("id=11 "))
        .expect("the block-space row");
    assert!(block.contains("space=Block(9)"), "{block}");
    assert!(block.contains("text_style=ANNO"), "{block}");
    let insert = objects
        .iter()
        .find(|message| message.contains("id=12 "))
        .expect("the INSERT row");
    assert!(insert.contains("block=9 name=DOOR"), "{insert}");
    let dimension = objects
        .iter()
        .find(|message| message.contains("id=13 "))
        .expect("the DIMENSION row");
    assert!(dimension.contains("dim_style=Standard"), "{dimension}");
    let hidden = objects
        .iter()
        .find(|message| message.contains("id=14 "))
        .expect("the hidden row");
    assert!(hidden.contains("visible=false"), "{hidden}");

    // Still read-only: no revision bump and no undo entry.
    let after = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();
    assert_eq!(after.revision(), before.revision());
    assert!(!app.can_undo(&document));
}

#[test]
fn list_refuses_an_empty_selection() {
    let (mut app, mut session, _document) = list_fixture();
    let error = app
        .execute(&mut session, command(CommandId::List, CommandPayload::None))
        .expect_err("an empty selection is refused");
    assert!(error.to_string().contains("list.selection"), "{error}");
}

#[test]
fn list_is_allowed_in_viewer_mode() {
    let (mut app, session, document) = list_fixture();
    let mut viewer = SessionState::new(document, AppMode::Viewer);
    viewer.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    assert!(app
        .execute(&mut viewer, command(CommandId::List, CommandPayload::None))
        .is_ok());
    // The fixture's Work session is untouched by the assertion above.
    assert_eq!(session.mode(), AppMode::Work);
}

fn status_fixture() -> (Application, SessionState, DocumentId) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name, visible) in [
        (LayerId(0), "0", true),
        (LayerId(3), "WALLS", true),
        (LayerId(4), "OFF", false),
        (LayerId(5), "FROZEN", false),
        (LayerId(6), "LOCKED", true),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
    }
    builder
        .set_layer_attributes(
            LayerId(4),
            LayerAttributes {
                off: true,
                ..Default::default()
            },
        )
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(5),
            LayerAttributes {
                frozen: true,
                ..Default::default()
            },
        )
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(6),
            LayerAttributes {
                locked: true,
                ..Default::default()
            },
        )
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(7),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(1),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
        .insert_linetype(LineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::continuous(),
            complex: false,
        })
        .unwrap();

    builder
        .insert_entity(line(1, point(0.0, 0.0), point(10.0, 10.0)))
        .unwrap();
    builder
        .insert_entity(line(2, point(10.0, 0.0), point(0.0, 10.0)))
        .unwrap();
    builder
        .insert_entity(entity(
            3,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
            LayerId(0),
            SpaceId::Paper(LayoutId(7)),
            "AcDbLine",
            None,
        ))
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(4)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            4,
            SemanticGeometry::Point(point(0.0, 0.0)),
            LayerId(0),
            SpaceId::Block(BlockId(2)),
            "AcDbPoint",
            None,
        ))
        .unwrap();
    let (app, mut session, document) = app_from_builder(builder);
    session.active_layer = LayerId(3);
    (app, session, document)
}

#[test]
fn status_reports_fixture_counts_and_honest_not_tracked_lines() {
    let (mut app, mut session, document) = status_fixture();
    let before = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Status, CommandPayload::None),
        )
        .expect("STATUS is admitted");

    assert_eq!(outcome.diagnostics[0].code, "status.summary");
    let summary = diagnostic(&outcome, "status.summary")[0];
    assert!(
        summary.contains("entities=4 (model=2 paper=1 block=1)"),
        "{summary}"
    );
    assert!(summary.contains(&format!("revision={}", before.revision().0)));

    let layers = diagnostic(&outcome, "status.layers")[0];
    assert!(layers.contains("layers=5"), "{layers}");
    assert!(layers.contains("off=1"), "{layers}");
    assert!(layers.contains("frozen=1"), "{layers}");
    assert!(layers.contains("locked=1"), "{layers}");

    let tables = diagnostic(&outcome, "status.tables")[0];
    assert!(tables.contains("blocks=1"), "{tables}");
    assert!(tables.contains("linetypes=1"), "{tables}");
    assert!(tables.contains("text_styles=1"), "{tables}");
    assert!(tables.contains("dimstyles=1"), "{tables}");
    assert!(tables.contains("mleader_styles=1"), "{tables}");
    assert!(tables.contains("layouts=1"), "{tables}");

    assert!(diagnostic(&outcome, "status.bounds")[0].contains("min="));
    assert!(diagnostic(&outcome, "status.active_layer")[0].contains("WALLS"));
    let session_line = diagnostic(&outcome, "status.session")[0];
    assert!(session_line.contains("mode=Work"), "{session_line}");
    assert!(session_line.contains("grid=true"), "{session_line}");
    assert!(session_line.contains("snap=false"), "{session_line}");
    assert!(session_line.contains("snap_grid=false"), "{session_line}");
    assert!(session_line.contains("ortho=false"), "{session_line}");
    assert!(session_line.contains("object_snap=true"), "{session_line}");
    assert!(diagnostic(&outcome, "status.zoom_history")[0].contains("zoom-history depth"));

    let not_tracked = diagnostic(&outcome, "status.not_tracked").join("\n");
    assert!(not_tracked.contains("creation/edit time"), "{not_tracked}");
    assert!(
        not_tracked.contains("limits and base point"),
        "{not_tracked}"
    );
    assert!(not_tracked.contains("memory usage"), "{not_tracked}");

    let after = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();
    assert_eq!(after.revision(), before.revision());
    assert!(!app.can_undo(&document));
}

#[test]
fn status_reports_empty_bounds_for_an_empty_drawing() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let (mut app, mut session, _document) = app_from_builder(builder);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Status, CommandPayload::None),
        )
        .expect("STATUS reports an empty drawing");
    assert!(diagnostic(&outcome, "status.summary")[0].contains("entities=0"));
    assert!(diagnostic(&outcome, "status.bounds")[0].contains("empty"));
}

#[test]
fn status_is_allowed_in_viewer_mode() {
    let (mut app, _session, document) = status_fixture();
    let mut viewer = SessionState::new(document, AppMode::Viewer);
    assert!(app
        .execute(
            &mut viewer,
            command(CommandId::Status, CommandPayload::None)
        )
        .is_ok());
}

fn fs_fixture() -> (Application, SessionState, DocumentId) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layer(Layer {
            id: LayerId(8),
            name: "HIDDEN".into(),
            visible: false,
        })
        .unwrap();
    // seed
    builder
        .insert_entity(line(1, point(0.0, 0.0), point(10.0, 10.0)))
        .unwrap();
    // crossing
    builder
        .insert_entity(line(2, point(0.0, 10.0), point(10.0, 0.0)))
        .unwrap();
    // far away
    builder
        .insert_entity(line(3, point(100.0, 100.0), point(110.0, 110.0)))
        .unwrap();
    // shares the seed's (10, 10) endpoint
    builder
        .insert_entity(line(4, point(10.0, 10.0), point(20.0, 10.0)))
        .unwrap();
    // crosses the seed at (5, 5) but is on a hidden layer
    builder
        .insert_entity(entity(
            5,
            SemanticGeometry::Line {
                start: point(2.0, 8.0),
                end: point(8.0, 2.0),
            },
            LayerId(8),
            SpaceId::Model,
            "AcDbLine",
            None,
        ))
        .unwrap();
    app_from_builder(builder)
}

#[test]
fn fs_adds_only_directly_touching_visible_entities() {
    let (mut app, mut session, document) = fs_fixture();
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    let before = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS is admitted");

    let applied = diagnostic(&outcome, "fs.applied");
    assert_eq!(applied.len(), 1);
    assert!(
        applied[0].contains("added 2 touching object(s)"),
        "{applied:?}"
    );
    assert!(session
        .selection
        .contains(&model_ref(document, EntityId(1))));
    assert!(session
        .selection
        .contains(&model_ref(document, EntityId(2))));
    assert!(session
        .selection
        .contains(&model_ref(document, EntityId(4))));
    assert!(
        !session
            .selection
            .contains(&model_ref(document, EntityId(3))),
        "the far line is not touched"
    );
    assert!(
        !session
            .selection
            .contains(&model_ref(document, EntityId(5))),
        "an entity on a hidden layer is excluded"
    );
    assert_eq!(session.selection.len(), 3, "seed retained, two added");

    let after = app
        .workspace
        .documents
        .get(&document)
        .unwrap()
        .drawing
        .clone();
    assert_eq!(after.revision(), before.revision());
    assert!(!app.can_undo(&document));
}

#[test]
fn fs_is_a_union_that_retains_the_seed() {
    let (mut app, mut session, document) = fs_fixture();
    // FS requires exactly one selected seed (see the refusal test below), so the
    // only "prior" ref it can retain is the seed itself. The result is still a
    // union, never a replacement: the seed stays selected and the touching
    // entities are appended.
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    let before = session.selection.clone();

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS is admitted");
    assert!(diagnostic(&outcome, "fs.applied")[0].contains("added 2 touching object(s)"));
    assert_eq!(session.selection.len(), 3);
    for prior in before.refs() {
        assert!(session.selection.contains(prior));
    }
}

#[test]
fn fs_with_nothing_touching_reports_fs_none_and_leaves_the_selection_unchanged() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    builder
        .insert_entity(line(2, point(50.0, 50.0), point(51.0, 50.0)))
        .unwrap();
    let (mut app, mut session, document) = app_from_builder(builder);
    session.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);

    let outcome = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect("FS reports no touch as a diagnostic, not an error");
    assert_eq!(diagnostic(&outcome, "fs.none").len(), 1);
    assert_eq!(session.selection.len(), 1);
    assert!(session
        .selection
        .contains(&model_ref(document, EntityId(1))));
}

#[test]
fn fs_refuses_empty_and_multi_object_selections() {
    let (mut app, mut session, document) = fs_fixture();
    let error = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect_err("no seed is refused");
    assert!(error.to_string().contains("fs.selection"), "{error}");

    session.selection = SelectionSet::from_refs([
        model_ref(document, EntityId(1)),
        model_ref(document, EntityId(2)),
    ]);
    let error = app
        .execute(&mut session, command(CommandId::Fs, CommandPayload::None))
        .expect_err("more than one seed is refused");
    assert!(error.to_string().contains("fs.selection"), "{error}");
}

#[test]
fn fs_is_allowed_in_viewer_mode() {
    let (mut app, _session, document) = fs_fixture();
    let mut viewer = SessionState::new(document, AppMode::Viewer);
    viewer.selection = SelectionSet::from_refs([model_ref(document, EntityId(1))]);
    assert!(app
        .execute(&mut viewer, command(CommandId::Fs, CommandPayload::None))
        .is_ok());
    assert_eq!(viewer.selection.len(), 3);
}
