//! `PRESSPULL` command contracts.
//!
//! The contracts use a **fake** [`SolidModeler`] that answers the `OffsetFace`
//! operation with the unit box, so the command layer is pinned independently of
//! the kernel. The real kernel face move is exercised by
//! `cad-kernel-solidcore/tests/offset_face.rs`.
//!
//! Pinned here: the in-place replacement (same id, one revision, one undo), the
//! face-ref → body collapse, the signed distance reaching the kernel unchanged,
//! the `presspull.applied` diagnostic, and every named refusal with **zero**
//! revision/undo change. `cad-app` never pre-judges the face: the kernel's
//! `solid.face_not_found` / `solid.offset_face_failed` answers pass through.

use super::*;
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer};
use cad_kernel_adapter::{
    ModelerCapabilities, ModelerPrimitives, SolidFaceSelector, SolidModeler, SolidOperation,
    SolidOutcome, SolidProduct, SolidRequest, SolidResult, SolidUnsupportedReason,
};
use std::sync::{Arc, Mutex};

use super::solids::{p, unit_box};

const BODY: u128 = 930;
const LINE: u128 = 931;

// ---- fixtures -------------------------------------------------------------

fn display_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

fn exact_box(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDb3dSolid".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Body3d(Box::new(Body3d {
            kind: BodyKind::Solid,
            exact: BodyExact::Exact(Box::new(unit_box())),
            display: BodyDisplay {
                mesh: display_mesh(),
                edges: Vec::new(),
                tolerance: Some(0.01),
                precision: Precision::Approximate {
                    error_bound: Some(0.01),
                },
                completeness: Completeness::Partial(vec!["test display cache".into()]),
            },
        })),
        draw_order: id as i64,
    }
}

fn line_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        draw_order: id as i64,
    }
}

fn face_ref(id: u128) -> SelectionRef {
    SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(id)
    }
}

fn app_with_entities(entities: Vec<DbEntity>) -> (Application, SessionState) {
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
    let (app, session, _) = app_from_builder(builder);
    (app, session)
}

fn stored_face_count(app: &Application, id: u128) -> usize {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the body exists");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("entity {id} is not a body");
    };
    body.exact_brep().expect("exact").shells[0].faces.len()
}

// ---- the fake modeler -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct OffsetCall {
    selector: Point3,
    distance: f64,
}

enum Answer {
    Moved,
    Unsupported(&'static str),
}

struct FakeOffsetModeler {
    answer: Answer,
    calls: Arc<Mutex<Vec<OffsetCall>>>,
}

impl SolidModeler for FakeOffsetModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.fake-offset-face".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["offset.face".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::default(),
            booleans: false,
            mass_properties: false,
            chamfer: false,
            offset_face: true,
            extrude_profile: false,
            fillet: false,
            shell: false,
            nurbs: false,
        }
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult> {
        let SolidOperation::OffsetFace {
            selector, distance, ..
        } = request.operation
        else {
            panic!("PRESSPULL must send an OffsetFace operation");
        };
        let SolidFaceSelector::NearestToPoint(point) = selector;
        self.calls.lock().expect("call log").push(OffsetCall {
            selector: point,
            distance,
        });
        let outcome = match self.answer {
            Answer::Moved => SolidOutcome::Success(SolidProduct::Body {
                brep: unit_box(),
                diagnostics: Vec::new(),
            }),
            Answer::Unsupported(code) => SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code,
                    operation: request.operation.family(),
                    detail: "the fake kernel declines".into(),
                },
            },
        };
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

fn with_modeler(app: &mut Application, answer: Answer) -> Arc<Mutex<Vec<OffsetCall>>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    app.set_solid_modeler(Some(Arc::new(FakeOffsetModeler {
        answer,
        calls: calls.clone(),
    })));
    calls
}

fn press_pull_command(body: SelectionRef, seed: Point3) -> (CommandId, CommandPayload) {
    (
        CommandId::Extended(ExtendedCommandId::PressPull),
        CommandPayload::PressPullFace { body, seed },
    )
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn set_distance(session: &mut SessionState, distance: f64) {
    session
        .set_tool_parameter(
            ToolParameter::PressPullDistance,
            ToolValue::Number(distance),
        )
        .expect("the distance is a legal stored value");
}

fn assert_wrote_nothing(app: &Application, before: Revision) {
    assert_eq!(revision(app), before, "a refusal writes nothing");
    assert!(!app.can_undo(&DocumentId(1)), "a refusal records no undo");
}

// ---- apply ----------------------------------------------------------------

#[test]
fn press_pull_moves_the_face_in_place_with_one_transaction_and_one_undo() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    let calls = with_modeler(&mut app, Answer::Moved);
    set_distance(&mut session, 2.5);
    let before_revision = revision(&app);
    let faces_before = stored_face_count(&app, BODY);
    let seed = p(0.5, 0.5, 1.0);

    let (id, payload) = press_pull_command(face_ref(BODY), seed);
    let outcome = execute(&mut app, &mut session, id, payload).expect("the face moves");

    assert_eq!(outcome.objects, vec![ObjectId(BODY)]);
    assert_eq!(revision(&app), Revision(before_revision.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    assert_eq!(
        stored_face_count(&app, BODY),
        faces_before,
        "same body shape"
    );

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].selector, seed,
        "the kernel gets the pick hit point"
    );
    assert_eq!(calls[0].distance, 2.5, "the typed distance is signed out");
    drop(calls);

    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(BODY))
        .expect("the row is still there");
    assert_eq!(entity.object.type_key, "AcDb3dSolid");
    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "presspull.applied")
        .expect("the success names the distance and seed");
    assert!(applied.message.contains("2.5"), "{}", applied.message);
    assert!(
        applied.message.contains("(0.5, 0.5, 1)"),
        "{}",
        applied.message
    );
}

#[test]
fn a_negative_distance_pushes_inward_and_reaches_the_kernel_signed() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    let calls = with_modeler(&mut app, Answer::Moved);
    set_distance(&mut session, -1.5);
    let (id, payload) = press_pull_command(ref_of(BODY), p(0.0, 0.0, 0.0));
    execute(&mut app, &mut session, id, payload).expect("the face moves");
    assert_eq!(calls.lock().unwrap()[0].distance, -1.5);
}

#[test]
fn a_face_ref_collapses_to_its_owning_body() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    let calls = with_modeler(&mut app, Answer::Moved);
    set_distance(&mut session, 1.0);
    let (id, payload) = press_pull_command(face_ref(BODY), p(0.0, 0.0, 1.0));
    let outcome = execute(&mut app, &mut session, id, payload).expect("the face moves");
    assert_eq!(outcome.objects, vec![ObjectId(BODY)]);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

// ---- refusals -------------------------------------------------------------

#[test]
fn an_unset_or_zero_distance_is_refused_by_name_and_writes_nothing() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    with_modeler(&mut app, Answer::Moved);
    let before = revision(&app);
    // The stored default is 0 (unset); it must not be executed as a no-op.
    let (id, payload) = press_pull_command(ref_of(BODY), p(0.0, 0.0, 1.0));
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("presspull.distance_invalid:"), "{text}");
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_typed_command_without_the_capture_payload_is_refused() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    with_modeler(&mut app, Answer::Moved);
    set_distance(&mut session, 1.0);
    let before = revision(&app);
    let id = CommandId::Extended(ExtendedCommandId::PressPull);
    let text = refusal_text(execute(&mut app, &mut session, id, CommandPayload::None).unwrap_err());
    assert!(text.starts_with("presspull.requires_payload:"), "{text}");
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_carrier_without_a_modeler_refuses_by_name() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    set_distance(&mut session, 1.0);
    let before = revision(&app);
    let (id, payload) = press_pull_command(ref_of(BODY), p(0.0, 0.0, 1.0));
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.contains("solid-modeler"), "{text}");
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_non_body_selection_is_refused_by_name() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY), line_entity(LINE)]);
    with_modeler(&mut app, Answer::Moved);
    set_distance(&mut session, 1.0);
    let before = revision(&app);
    let (id, payload) = press_pull_command(ref_of(LINE), p(0.0, 0.0, 0.0));
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("presspull.not_a_solid:"), "{text}");
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_kernel_refusal_passes_through_unchanged() {
    let (mut app, mut session) = app_with_entities(vec![exact_box(BODY)]);
    with_modeler(&mut app, Answer::Unsupported("solid.face_not_found"));
    set_distance(&mut session, 1.0);
    let before = revision(&app);
    let (id, payload) = press_pull_command(ref_of(BODY), p(0.0, 0.0, 1.0));
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("solid.face_not_found:"), "{text}");
    assert_wrote_nothing(&app, before);
}
