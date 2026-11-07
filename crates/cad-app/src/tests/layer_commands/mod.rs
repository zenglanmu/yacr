//! Layer **selection** command contracts (AutoCAD 2026).
//!
//! These commands are **pre-selection based** in this build: the fixed capture
//! machine cannot ask AutoCAD's *Select objects* prompt, so the current session
//! selection is the whole input (`LAYCUR` here; the rest of the run is added
//! beside it). Pinned per command: the success shape, one transaction / one
//! undo entry where entity rows are written, **no** undo entry for layer-table
//! edits, every refusal leaving revision + history untouched, the
//! addressability preflight, the current-layer refusal for LAYFRZ, the locked
//! destination refusal and (later) SETBYLAYER's preserved fields.

use super::*;
use cad_db::{
    DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineType, EntityLineWeight,
    EntityRenderAttributes, EntityTransparency, Layer, LayerAttributes, Layout, LinetypePattern,
};

fn line(id: u128, layer: LayerId, y: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, y),
            end: point(10.0, y),
        },
        draw_order: id as i64,
    }
}

/// A work-mode application with exactly `layers` and `entities`.
fn app_with(
    layers: Vec<(LayerId, &str, LayerAttributes)>,
    entities: Vec<DbEntity>,
) -> (Application, SessionState) {
    app_with_display(layers, entities, Vec::new())
}

/// The same fixture, with importer-resolved display attributes per entity.
fn app_with_display(
    layers: Vec<(LayerId, &str, LayerAttributes)>,
    entities: Vec<DbEntity>,
    display: Vec<(EntityId, EntityRenderAttributes)>,
) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name, attributes) in layers {
        let visible = attributes.is_visible();
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
        builder.set_layer_attributes(id, attributes).unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    for (id, attributes) in display {
        builder
            .set_entity_render_attributes(id, attributes)
            .unwrap();
    }
    let drawing = builder.finish().unwrap();

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

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(entities.iter().copied().map(ref_of).collect()),
    )
    .unwrap();
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn state(app: &Application) -> (Revision, usize) {
    (revision(app), undo_depth(app))
}

fn assert_untouched(app: &Application, before: (Revision, usize)) {
    assert_eq!(revision(app), before.0, "a refusal bumps no revision");
    assert_eq!(undo_depth(app), before.1, "a refusal records no undo entry");
}

fn refusal(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn layer_attributes(app: &Application, layer: LayerId) -> LayerAttributes {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .layer_attributes(layer)
}

fn entity_layer(app: &Application, entity: u128) -> LayerId {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .layer
}

fn diagnostic_codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

mod laycur;
mod layfrz;
mod laylck;
mod laymcur;
mod laythw;
mod setbylayer;
