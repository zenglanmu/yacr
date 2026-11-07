//! `LIST`, `STATUS` and `FS` contract tests (read-only inquiry commands).
//!
//! Pinned here: LIST's per-entity report (including space and display
//! properties), STATUS's fixture counts and honest `not tracked` lines, FS's
//! direct-touch union, the Viewer-mode admission of all three, and the fact that
//! none of them writes the database, bumps a revision or records undo.

use super::*;
use crate::selection::model_ref;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

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
