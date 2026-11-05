//! Vendor-derived pick catalogue contract tests (ADR-0028 D1/D2).
//!
//! These pin the two rules that make TCH entities usable without writing them to
//! `cad-db`:
//!
//! * only a **committed** database entity is selectable (a staged transaction
//!   row is not, and an API-created entity carries no `source_handle`);
//! * an `Opaque` entity is offered through its settled vendor stand-in, and is
//!   still honestly `opaque` / refused when there is no stand-in.

use super::*;
use crate::vendor::{VendorEntityPrimitives, VendorPrimitive, VendorRepresentationStore};
use crate::{
    AppMode, Application, Command, CommandId, CommandPayload, SessionState, Viewport, ViewportId,
};
use cad_db::{DrawingDatabaseBuilder, Layer};
use cad_domain::{DocumentIdentity, TransactionId, UnitContext};
use std::sync::Arc;

const DOC: DocumentId = DocumentId(1);
const VENDOR_HANDLE: &str = "2C";
const VENDOR_ENTITY: u128 = 20;

fn options() -> PickOptions {
    PickOptions::new(0.05).unwrap()
}

/// A ray along +x through y = 0.
fn ray_x() -> Ray3 {
    Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(1.0, 0.0, 0.0),
    }
}

fn vertical_line(x: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p(x, -1.0, 0.0),
        end: p(x, 1.0, 0.0),
    }
}

fn opaque_entity(id: u128, handle: Option<&str>, draw_order: i64) -> DbEntity {
    let mut entity = model_entity(
        id,
        SemanticGeometry::Opaque {
            type_key: "TCH_COLUMN".into(),
            version: 0,
            payload: Vec::new(),
        },
        draw_order,
    );
    entity.object.source_handle = handle.map(str::to_string);
    entity
}

fn database_with(entities: Vec<DbEntity>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    builder.finish().unwrap()
}

/// A store whose single entity (`handle 0x2C`) owns `primitives`.
fn store(revision: u64, primitives: Vec<VendorPrimitive>) -> VendorRepresentationStore {
    VendorRepresentationStore::new(
        revision,
        vec![VendorEntityPrimitives {
            handle: 0x2C,
            primitives,
        }],
    )
}

/// A vertical derived line crossing the +x pick ray at the origin.
fn crossing_line() -> VendorPrimitive {
    VendorPrimitive::Lines(vec![p(0.0, -1.0, 0.0), p(0.0, 1.0, 0.0)])
}

fn select_all_app(
    entities: Vec<DbEntity>,
    vendor: Option<VendorRepresentationStore>,
) -> (Application, SessionState) {
    let drawing = database_with(entities);
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        crate::Document {
            id: DOC,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([28u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    if let Some(store) = vendor {
        app.set_vendor_representations(Some(Arc::new(store)));
    }
    (app, SessionState::new(DOC, AppMode::Work))
}

fn select_all_command() -> Command {
    Command {
        schema_version: 1,
        id: CommandId::SelectAll,
        document: DOC,
        viewport: ViewportId(1),
        payload: CommandPayload::None,
    }
}

#[test]
fn a_committed_database_entity_is_in_the_pick_catalogue() {
    let db = database_with(vec![model_entity(1, vertical_line(0.0), 0)]);
    let items = drawing_pick_items(&db, DOC);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source.entity, EntityId(1));
    assert_eq!(items[0].source.document, DOC);
    assert!(items[0].source.instance.0.is_empty());
    let report = pick_ray(&db, DOC, &ray_x(), &options()).unwrap();
    assert_eq!(
        report.hit.expect("committed line hits").source.entity,
        EntityId(1)
    );
}

/// D1: a staged transaction row is not in the database, so it is not in the
/// catalogue; once committed it is, with no source handle (never fabricated).
#[test]
fn a_staged_entity_is_not_pickable_until_committed_and_then_has_no_handle() {
    let mut db = database_with(vec![model_entity(1, vertical_line(10.0), 0)]);
    assert_eq!(drawing_pick_items(&db, DOC).len(), 1);
    let id = db.allocate_entity_id();
    let staged_at = Ray3 {
        origin: p(15.0, 0.0, 0.0),
        direction: p(1.0, 0.0, 0.0),
    };

    // Stage, then drop: a rolled-back row leaves no database state. (The
    // transaction owns the mutable database, so the committed view is the only
    // readable one while it is alive; a sibling snapshot would invent state.)
    {
        let mut tx = db
            .begin_drawing_transaction("pick-staging", TransactionId(1))
            .unwrap();
        tx.insert_entity(model_entity(id.0, vertical_line(20.0), 1))
            .unwrap();
        assert_eq!(tx.staged_len(), 1);
    }
    assert!(
        db.entity(id).is_none(),
        "a rolled-back stage is not an entity"
    );
    assert_eq!(drawing_pick_items(&db, DOC).len(), 1);
    assert!(pick_ray(&db, DOC, &staged_at, &options())
        .unwrap()
        .hit
        .is_none());

    // Commit for real: now the entity is selectable.
    {
        let mut tx = db
            .begin_drawing_transaction("pick-commit", TransactionId(2))
            .unwrap();
        tx.insert_entity(model_entity(id.0, vertical_line(20.0), 1))
            .unwrap();
        tx.commit().unwrap();
    }
    let items = drawing_pick_items(&db, DOC);
    assert_eq!(items.len(), 2);
    let added = items
        .iter()
        .find(|item| item.source.entity == id)
        .expect("the committed entity is in the catalogue");
    assert_eq!(added.source.entity, id);
    assert!(added.source.sub_element.is_none());
    assert!(
        db.entity(id).unwrap().object.source_handle.is_none(),
        "an API-created committed entity has no source handle"
    );
    assert_eq!(
        pick_ray(&db, DOC, &staged_at, &options())
            .unwrap()
            .hit
            .expect("the committed entity hits")
            .source
            .entity,
        id
    );
}

/// D2: the same committed `Opaque` entity is pickable through its vendor
/// stand-in and honestly `opaque` without one.
#[test]
fn a_vendor_backed_opaque_entity_is_pickable_and_stays_opaque_without_the_store() {
    let db = database_with(vec![opaque_entity(VENDOR_ENTITY, Some(VENDOR_HANDLE), 0)]);
    let vendor = store(7, vec![crossing_line()]);

    let items = drawing_pick_items_with_vendor(&db, DOC, Some(&vendor));
    assert_eq!(items.len(), 1);
    assert!(
        matches!(items[0].geometry, SemanticGeometry::Compound(_)),
        "{:?}",
        items[0].geometry
    );
    assert_eq!(items[0].geometry_source, GeometrySource::ProxyCache);
    assert_eq!(items[0].source.entity, EntityId(VENDOR_ENTITY));
    assert_eq!(items[0].source.document, DOC);
    assert!(items[0].source.sub_element.is_none());

    let report = pick_ray_with_vendor(&db, DOC, &ray_x(), &options(), Some(&vendor)).unwrap();
    assert_eq!(
        report.hit.expect("vendor stand-in hit").source.entity,
        EntityId(VENDOR_ENTITY)
    );
    assert!(report.skipped.is_empty());

    // No snapshot: the item is still `Opaque`, and the report says so.
    let plain_items = drawing_pick_items(&db, DOC);
    assert!(matches!(
        plain_items[0].geometry,
        SemanticGeometry::Opaque { .. }
    ));
    assert_eq!(plain_items[0].geometry_source, GeometrySource::Analytic);
    let plain = pick_ray(&db, DOC, &ray_x(), &options()).unwrap();
    assert!(plain.hit.is_none());
    assert_eq!(plain.skipped.len(), 1);
    assert_eq!(plain.skipped[0].reason, "opaque");
    assert_eq!(plain.skipped[0].source.entity, EntityId(VENDOR_ENTITY));

    // Block-only primitives are a documented limitation: no synthesized
    // geometry, so the entity stays unpickable rather than faking a hit.
    let block_only = store(
        8,
        vec![VendorPrimitive::Block {
            block: 0x10,
            origin: p(0.0, 0.0, 0.0),
            scale: p(1.0, 1.0, 1.0),
            rotation: 0.0,
        }],
    );
    let block_items = drawing_pick_items_with_vendor(&db, DOC, Some(&block_only));
    assert!(matches!(
        block_items[0].geometry,
        SemanticGeometry::Opaque { .. }
    ));
    assert!(
        pick_ray_with_vendor(&db, DOC, &ray_x(), &options(), Some(&block_only))
            .unwrap()
            .hit
            .is_none()
    );
}

/// D2: the vendor snapshot changes without a DB revision, so the cached index
/// must key on the pass token. Otherwise the old broad phase hides the entity.
#[test]
fn a_vendor_revision_change_invalidates_the_cached_pick_index() {
    let db = database_with(vec![opaque_entity(VENDOR_ENTITY, Some(VENDOR_HANDLE), 0)]);
    let far = store(
        1,
        vec![VendorPrimitive::Lines(vec![
            p(0.0, 100.0, 0.0),
            p(100.0, 100.0, 0.0),
        ])],
    );
    let near = store(2, vec![crossing_line()]);

    let first = pick_ray_with_vendor(&db, DOC, &ray_x(), &options(), Some(&far)).unwrap();
    assert!(
        first.hit.is_none(),
        "the far stand-in is outside the broad phase"
    );
    assert_eq!(
        cached_pick_index(&db, DOC, Some(&far))
            .expect("index")
            .vendor_revision(),
        Some(1)
    );

    // Same scene identity and document, new pass token.
    let second = pick_ray_with_vendor(&db, DOC, &ray_x(), &options(), Some(&near)).unwrap();
    assert_eq!(
        second.hit.expect("the new pass is pickable").source.entity,
        EntityId(VENDOR_ENTITY)
    );
    assert_eq!(
        cached_pick_index(&db, DOC, Some(&near))
            .expect("rebuilt index")
            .vendor_revision(),
        Some(2)
    );

    // Removing the snapshot is also a key change, never a stale hit.
    assert!(pick_ray(&db, DOC, &ray_x(), &options())
        .unwrap()
        .hit
        .is_none());
    assert_eq!(
        cached_pick_index(&db, DOC, None)
            .expect("index")
            .vendor_revision(),
        None
    );
}

/// D2/§4: `SELECTALL` enumerates a vendor-backed opaque entity and still
/// refuses a truly unsupported one (the same predicate the catalogue uses).
#[test]
fn select_all_includes_a_vendor_backed_opaque_entity_and_refuses_an_unsupported_one() {
    let (mut app, mut session) = select_all_app(
        vec![
            model_entity(1, vertical_line(0.0), 0),
            opaque_entity(VENDOR_ENTITY, Some(VENDOR_HANDLE), 1),
        ],
        Some(store(7, vec![crossing_line()])),
    );
    app.execute(&mut session, select_all_command())
        .expect("a vendor-backed opaque entity is enumerable");
    let selected: Vec<EntityId> = session.selection.refs().iter().map(|r| r.entity).collect();
    assert!(selected.contains(&EntityId(1)));
    assert!(
        selected.contains(&EntityId(VENDOR_ENTITY)),
        "the opaque entity is selected through its stand-in: {selected:?}"
    );

    // No snapshot: the same committed opaque entity is an explicit refusal.
    let (mut app, mut session) = select_all_app(
        vec![opaque_entity(VENDOR_ENTITY, Some(VENDOR_HANDLE), 0)],
        None,
    );
    assert!(matches!(
        app.execute(&mut session, select_all_command()),
        Err(CadError::Unsupported(_))
    ));
    assert!(
        session.selection.is_empty(),
        "a refusal never publishes a partial selection"
    );

    // A snapshot with only block primitives synthesizes nothing, so the entity
    // is not "supported" for selection either.
    let (mut app, mut session) = select_all_app(
        vec![opaque_entity(VENDOR_ENTITY, Some(VENDOR_HANDLE), 0)],
        Some(store(
            8,
            vec![VendorPrimitive::Block {
                block: 0x10,
                origin: p(0.0, 0.0, 0.0),
                scale: p(1.0, 1.0, 1.0),
                rotation: 0.0,
            }],
        )),
    );
    assert!(matches!(
        app.execute(&mut session, select_all_command()),
        Err(CadError::Unsupported(_))
    ));
}
