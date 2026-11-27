//! `FOLLOWME` host contract: the zero-point tool route and the one-transaction
//! commit.
//!
//! Drives the real `HostCore` request path (no UI, no GPU): `BeginDraw(FollowMe)`
//! must arm the zero-point capture, and `Confirm` must dispatch the command that
//! inserts exactly one `AcDbSubDMesh` row while the profile and path stay put.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, EntityId, InstancePath, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId,
};

const PROFILE: u128 = 960;
const PATH: u128 = 961;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn polyline(id: u128, points: Vec<Point3>, closed: bool) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Polyline {
            points,
            bulges: vec![0.0; 4],
            closed,
            width: 0.0,
        },
        draw_order: id as i64,
    }
}

fn install(core: &HostCore, entities: Vec<DbEntity>) {
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
    let drawing = Arc::new(builder.finish().unwrap());
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    controller
        .application
        .workspace
        .documents
        .get_mut(&document)
        .expect("the host always has a document")
        .drawing = drawing;
}

fn select(core: &HostCore, entities: &[u128]) {
    let document = core.controller.borrow().document_id;
    core.controller
        .borrow_mut()
        .session
        .selection
        .replace(entities.iter().map(|id| SelectionRef {
            document,
            entity: EntityId(*id),
            instance: InstancePath::default(),
            sub_element: None,
        }));
}

fn count(core: &HostCore) -> usize {
    core.controller
        .borrow()
        .drawing()
        .expect("a drawing is open")
        .entity_count()
}

#[test]
fn begin_draw_arms_the_zero_point_follow_me_capture() {
    let core = core();
    install(
        &core,
        vec![
            polyline(
                PROFILE,
                vec![
                    p3(0.0, 0.0, 0.0),
                    p3(1.0, 0.0, 0.0),
                    p3(1.0, 1.0, 0.0),
                    p3(0.0, 1.0, 0.0),
                ],
                true,
            ),
            polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
        ],
    );
    select(&core, &[PROFILE, PATH]);
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::FollowMe)),
    )
    .expect("the tool starts on a profile and a path");
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        Some(DrawToolKind::FollowMe)
    );
    // No pick: the zero-point capture is armed with nothing captured.
    let snapshot = core
        .controller
        .borrow()
        .session
        .draw_preview()
        .expect("the capture publishes a preview");
    assert!(snapshot.points.is_empty(), "no point is captured");
}

#[test]
fn confirm_inserts_one_mesh_row_and_keeps_the_inputs() {
    let core = core();
    install(
        &core,
        vec![
            polyline(
                PROFILE,
                vec![
                    p3(0.0, 0.0, 0.0),
                    p3(1.0, 0.0, 0.0),
                    p3(1.0, 1.0, 0.0),
                    p3(0.0, 1.0, 0.0),
                ],
                true,
            ),
            polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
        ],
    );
    select(&core, &[PROFILE, PATH]);
    let before = count(&core);
    let before_revision = core.controller.borrow().drawing().unwrap().revision();
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::FollowMe)),
    )
    .expect("the tool starts");
    let outcome = core
        .controller
        .borrow_mut()
        .confirm_draw()
        .expect("the sweep commits");
    assert_eq!(count(&core), before + 1, "exactly one mesh row is created");
    assert_eq!(outcome.objects.len(), 1, "one created row");
    let created = outcome.objects[0];
    let drawing = core.controller.borrow();
    let drawing = drawing.drawing().expect("a drawing");
    assert!(
        drawing.entity(EntityId(PROFILE)).is_some(),
        "the profile row remains"
    );
    assert!(
        drawing.entity(EntityId(PATH)).is_some(),
        "the path row remains"
    );
    let entity = drawing
        .entity(EntityId(created.0))
        .expect("the created row");
    assert_eq!(entity.object.type_key, "AcDbSubDMesh");
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(
        drawing.revision(),
        Revision(before_revision.0 + 1),
        "one transaction"
    );
}
