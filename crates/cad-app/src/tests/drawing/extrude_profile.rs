//! `EXTRUDE` command contracts.
//!
//! A **fake** modeler answers `ExtrudeProfile` with the unit box, so the command
//! layer (profile validation, direction/sign mapping, one-transaction insert) is
//! pinned independently of the kernel. The real extrusion is exercised by
//! `cad-kernel-solidcore/tests/extrude_profile.rs`.
//!
//! Every refusal below happens before the transaction: zero revision, zero undo,
//! and the profile rows stay exactly as they were.

use super::*;
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer};
use cad_kernel_adapter::{
    ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOperation, SolidOutcome,
    SolidProduct, SolidRequest, SolidResult,
};
use std::sync::{Arc, Mutex};

use super::solids::{p, unit_box};

const LINE_A: u128 = 940;
const LINE_B: u128 = 941;
const LINE_C: u128 = 942;
const LINE_D: u128 = 943;
const ARC: u128 = 944;

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
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

fn line(id: u128, start: Point3, end: Point3) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, id as i64)
}

/// A 2 × 3 rectangle at `z`, as four LINE rows.
fn rectangle(z: f64) -> Vec<DbEntity> {
    vec![
        line(LINE_A, p(0.0, 0.0, z), p(2.0, 0.0, z)),
        line(LINE_B, p(2.0, 0.0, z), p(2.0, 3.0, z)),
        line(LINE_C, p(2.0, 3.0, z), p(0.0, 3.0, z)),
        line(LINE_D, p(0.0, 3.0, z), p(0.0, 0.0, z)),
    ]
}

fn refs() -> Vec<SelectionRef> {
    [LINE_A, LINE_B, LINE_C, LINE_D]
        .into_iter()
        .map(ref_of)
        .collect()
}

fn command_with_refs(refs: Vec<SelectionRef>) -> (CommandId, CommandPayload) {
    (
        CommandId::Extended(ExtendedCommandId::Extrude),
        CommandPayload::ExtrudeProfile(refs),
    )
}

#[derive(Debug, Clone, PartialEq)]
struct ExtrudeCall {
    curves: usize,
    direction: Point3,
    distance: f64,
}

struct FakeExtrudeModeler {
    calls: Arc<Mutex<Vec<ExtrudeCall>>>,
}

impl SolidModeler for FakeExtrudeModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.fake-extrude".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["extrude.profile".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::default(),
            booleans: false,
            mass_properties: false,
            chamfer: false,
            offset_face: false,
            extrude_profile: true,
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
        let SolidOperation::ExtrudeProfile {
            profile,
            direction,
            distance,
        } = request.operation
        else {
            panic!("EXTRUDE must send an ExtrudeProfile operation");
        };
        self.calls.lock().unwrap().push(ExtrudeCall {
            curves: profile.len(),
            direction,
            distance,
        });
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome: SolidOutcome::Success(SolidProduct::Body {
                brep: unit_box(),
                diagnostics: Vec::new(),
            }),
        })
    }
}

fn with_modeler(app: &mut Application) -> Arc<Mutex<Vec<ExtrudeCall>>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    app.set_solid_modeler(Some(Arc::new(FakeExtrudeModeler {
        calls: calls.clone(),
    })));
    calls
}

fn set_distance(session: &mut SessionState, distance: f64) {
    session
        .set_tool_parameter(ToolParameter::ExtrudeDistance, ToolValue::Number(distance))
        .expect("the distance is a legal stored value");
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

#[test]
fn extrude_inserts_one_new_solid_and_keeps_the_profile() {
    let (mut app, mut session) = app_with(rectangle(0.0));
    let calls = with_modeler(&mut app);
    set_distance(&mut session, 5.0);
    let before = revision(&app);
    let (id, payload) = command_with_refs(refs());
    let outcome = execute(&mut app, &mut session, id, payload).expect("the profile extrudes");

    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    assert_eq!(outcome.objects.len(), 1);
    let created = outcome.objects[0];
    let profile_objects: Vec<ObjectId> = [LINE_A, LINE_B, LINE_C, LINE_D]
        .into_iter()
        .map(ObjectId)
        .collect();
    assert!(
        !profile_objects.contains(&created),
        "the new body is not a profile row"
    );
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(created.0))
        .expect("the new body row exists");
    assert_eq!(entity.object.type_key, "AcDb3dSolid");
    // The four profile rows are untouched.
    for profile in refs() {
        assert!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(profile.entity)
                .is_some(),
            "profile row {} remains",
            profile.entity.0
        );
    }

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].curves, 4);
    assert_eq!(calls[0].distance, 5.0, "the magnitude, not the sign");
    assert_eq!(calls[0].direction.z, 1.0, "positive extrudes along +Z");
    drop(calls);

    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "extrude.applied")
        .expect("the success names the profile and distance");
    assert!(applied.message.contains("5"), "{}", applied.message);
}

#[test]
fn a_negative_distance_extrudes_along_minus_z() {
    let (mut app, mut session) = app_with(rectangle(0.0));
    let calls = with_modeler(&mut app);
    set_distance(&mut session, -2.5);
    let (id, payload) = command_with_refs(refs());
    execute(&mut app, &mut session, id, payload).expect("the profile extrudes");
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].distance, 2.5);
    assert_eq!(calls[0].direction.z, -1.0);
}

#[test]
fn an_open_chain_is_refused_by_name_and_writes_nothing() {
    let (mut app, mut session) = app_with(vec![
        line(LINE_A, p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)),
        line(LINE_B, p(2.0, 0.0, 0.0), p(2.0, 3.0, 0.0)),
    ]);
    with_modeler(&mut app);
    set_distance(&mut session, 1.0);
    let before = revision(&app);
    let (id, payload) = command_with_refs(vec![ref_of(LINE_A), ref_of(LINE_B)]);
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("extrude.profile_not_closed:"), "{text}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_arc_profile_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![ent(
        ARC,
        SemanticGeometry::Arc {
            center: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 1.0,
            start: 0.0,
            sweep: std::f64::consts::PI,
        },
        ARC as i64,
    )]);
    with_modeler(&mut app);
    set_distance(&mut session, 1.0);
    let (id, payload) = command_with_refs(vec![ref_of(ARC)]);
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("extrude.profile_unsupported:"), "{text}");
}

#[test]
fn a_non_planar_chain_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![
        line(LINE_A, p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)),
        line(LINE_B, p(2.0, 0.0, 0.0), p(2.0, 3.0, 1.0)),
        line(LINE_C, p(2.0, 3.0, 1.0), p(0.0, 3.0, 1.0)),
        line(LINE_D, p(0.0, 3.0, 1.0), p(0.0, 0.0, 0.0)),
    ]);
    with_modeler(&mut app);
    set_distance(&mut session, 1.0);
    let (id, payload) = command_with_refs(refs());
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("extrude.not_planar:"), "{text}");
}

#[test]
fn a_zero_distance_is_refused_by_name() {
    let (mut app, mut session) = app_with(rectangle(0.0));
    with_modeler(&mut app);
    let before = revision(&app);
    let (id, payload) = command_with_refs(refs());
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.starts_with("extrude.distance_invalid:"), "{text}");
    assert_eq!(revision(&app), before);
}

#[test]
fn a_carrier_without_a_modeler_refuses_by_name() {
    let (mut app, mut session) = app_with(rectangle(0.0));
    set_distance(&mut session, 1.0);
    let (id, payload) = command_with_refs(refs());
    let text = refusal_text(execute(&mut app, &mut session, id, payload).unwrap_err());
    assert!(text.contains("solid-modeler"), "{text}");
}

// ---- the Push/Pull entry's read-only profile predicate ---------------------
//
// The host arms `EXTRUDE` when Push/Pull starts on this selection. The
// predicate must accept exactly the rows the command would accept, and it must
// not write anything: these tests read it on a live application and then assert
// the revision and undo stack are untouched.

fn closed_rect(id: u128) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Polyline {
            points: vec![
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(2.0, 3.0, 0.0),
                p(0.0, 3.0, 0.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
            width: 0.0,
        },
        id as i64,
    )
}

#[test]
fn a_closed_polyline_is_a_push_pull_profile_and_the_probe_writes_nothing() {
    let (app, _session) = app_with(vec![closed_rect(LINE_A)]);
    let before = revision(&app);
    assert!(app.closed_profile_selection(DocumentId(1), &[ref_of(LINE_A)]));
    assert_eq!(revision(&app), before, "the probe is a pure read");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_line_ring_is_a_push_pull_profile() {
    let (app, _session) = app_with(rectangle(0.0));
    assert!(app.closed_profile_selection(DocumentId(1), &refs()));
}

#[test]
fn a_circle_is_a_push_pull_profile_but_an_open_chain_is_not() {
    let (app, _session) = app_with(vec![
        ent(
            LINE_A,
            SemanticGeometry::Circle {
                center: p(1.0, 1.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 1.0,
            },
            LINE_A as i64,
        ),
        line(LINE_B, p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)),
    ]);
    assert!(app.closed_profile_selection(DocumentId(1), &[ref_of(LINE_A)]));
    assert!(!app.closed_profile_selection(DocumentId(1), &[ref_of(LINE_B)]));
    assert!(
        !app.closed_profile_selection(DocumentId(1), &[]),
        "an empty selection is not a profile"
    );
}

#[test]
fn a_sub_element_or_instance_reference_is_never_a_profile() {
    let (app, _session) = app_with(vec![closed_rect(LINE_A)]);
    let mut face = ref_of(LINE_A);
    face.sub_element = Some(SubElementId {
        source_key: "face-0".into(),
        topology_revision: Revision(0),
    });
    assert!(!app.closed_profile_selection(DocumentId(1), &[face]));

    let mut instance = ref_of(LINE_A);
    instance.instance = InstancePath(vec![EntityId(77)]);
    assert!(!app.closed_profile_selection(DocumentId(1), &[instance]));
}
