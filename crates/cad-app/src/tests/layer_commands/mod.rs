//! Layer **selection** command contracts (AutoCAD 2026).
//!
//! These commands are **pre-selection based** in this build: the fixed capture
//! machine cannot ask AutoCAD's *Select objects* prompt, so the current session
//! selection is the whole input (`LAYCUR` here; the rest of the run is added
//! beside it). Pinned per command: the success shape, one transaction / one
//! undo entry where entity rows are written, **one** undo entry for layer-table
//! edits, every refusal leaving revision + history untouched, the
//! addressability preflight, the current-layer refusal for LAYFRZ, the locked
//! destination refusal and (later) SETBYLAYER's preserved fields.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineType,
    EntityLineWeight, EntityRenderAttributes, EntityTransparency, Layer, LayerAttributes, Layout,
    LinetypePattern,
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
    app_with_database(builder.finish().unwrap())
}

/// Wrap one already-built database (layouts, blocks and all) in a work-mode
/// application, so a test can build the fixture its command actually needs.
fn app_with_database(drawing: DrawingDatabase) -> (Application, SessionState) {
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
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

/// A **nested pick**: `member` inside the block displayed by `insert`, exactly
/// the ref shape the picker records for a block member.
fn nested_ref(insert: u128, member: u128) -> SelectionRef {
    SelectionRef {
        instance: InstancePath(vec![EntityId(insert)]),
        ..ref_of(member)
    }
}

/// A drawing whose selection layers have members in **every** space, so a
/// layer-table command can be pinned against the paper-space and block-member
/// picks the real machine exposed.
///
/// Layers `0`/`WALLS`/`DOORS`; layout `Sheet1` with **two** viewports, each
/// seeded with one of `overrides`; block `0` holding entities 5 (on `WALLS`) and
/// 6 (on layer `0`, so it is displayed on its `INSERT`'s layer). Entities: 1 in
/// model space on `WALLS`, 2 in paper space on `DOORS`, 3 in model space on
/// `DOORS` (the control), 4 = the model `INSERT` of block `0` on `DOORS`.
fn multi_space_app(overrides: [Vec<(LayerId, bool)>; 2]) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: overrides
                .into_iter()
                .map(|overrides| cad_db::PaperViewport {
                    clip: vec![
                        point(0.0, 0.0),
                        point(100.0, 0.0),
                        point(100.0, 50.0),
                        point(0.0, 50.0),
                    ],
                    model_to_paper: Transform3::scale(100.0),
                    completeness: Completeness::Complete,
                    layer_overrides: overrides.into_iter().collect(),
                })
                .collect(),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(5), EntityId(6)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder.insert_entity(line(1, LayerId(3), 0.0)).unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Paper(LayoutId(1)),
            ..line(2, LayerId(5), 5.0)
        })
        .unwrap();
    builder.insert_entity(line(3, LayerId(5), 10.0)).unwrap();
    builder
        .insert_entity(DbEntity {
            geometry: SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::identity(),
            },
            ..line(4, LayerId(5), 20.0)
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Block(BlockId(0)),
            ..line(5, LayerId(3), 8.0)
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Block(BlockId(0)),
            ..line(6, LayerId(0), 9.0)
        })
        .unwrap();
    app_with_database(builder.finish().unwrap())
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

/// Replace the selection with refs that cannot be spelled as a bare entity id
/// (a nested pick carries its `InstancePath`).
fn select_refs(app: &mut Application, session: &mut SessionState, refs: Vec<SelectionRef>) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(refs),
    )
    .unwrap();
}

/// One paper viewport's per-layer override map.
fn viewport_overrides(
    app: &Application,
    layout: LayoutId,
    index: usize,
) -> std::collections::BTreeMap<LayerId, bool> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .layout(layout)
        .expect("the layout must exist")
        .viewports[index]
        .layer_overrides
        .clone()
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
mod laydel;
mod layerpmode;
mod layerstate;
mod layfrz;
mod laylck;
mod laymch;
mod laymcur;
mod laymrg;
mod laythw;
mod laywalk;
mod ltscale;
mod setbylayer;
