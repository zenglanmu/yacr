//! ADR-0036 phase C2 **strict result gate**: a boolean whose folded B-rep is not
//! one connected shell must be refused, not committed.
//!
//! `UNION` of two disjoint boxes is the reachable case: `solidcore` answers with a
//! valid multi-lump body (two bounded shells), which this build stores as **one**
//! connected shell per body. The worker's representability gate refuses it with
//! `solid.boolean_result_not_single_shell` before the host can stage anything, so
//! the operands survive, no revision is bumped and no undo record is added.
//!
//! **Evidence boundary**: Linux x86-64 contract on the real `solidcore` modeler
//! (no GPU, window or carrier). It asserts the refusal path; it is not a visual,
//! pixel, performance or cross-carrier claim.

use std::sync::Arc;

use cad_app::host::{HostController, SolidBooleanPoll};
use cad_app::{Command, CommandId, CommandPayload, ExtendedCommandId, SelectionSet};
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, ViewportId};
use cad_kernel_adapter::SolidModeler;
use cad_kernel_solidcore::{SolidCoreModeler, SolidCoreTessellator};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn command(
    document: DocumentId,
    viewport: ViewportId,
    id: CommandId,
    payload: CommandPayload,
) -> Command {
    Command {
        schema_version: 1,
        id,
        document,
        viewport,
        payload,
    }
}

fn modeler_controller() -> HostController {
    let mut controller =
        HostController::with_blank_document([800.0, 600.0]).expect("blank document");
    let modeler: Arc<dyn SolidModeler> = Arc::new(SolidCoreModeler::new());
    controller.application.set_solid_modeler(Some(modeler));
    controller
        .application
        .set_solid_tessellator(Some(Arc::new(SolidCoreTessellator::new())));
    controller
}

fn solid_count(controller: &HostController) -> usize {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entities()
        .filter(|entity| entity.object.type_key == "AcDb3dSolid")
        .count()
}

fn create_box(controller: &mut HostController, min: Point3, max: Point3) -> EntityId {
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ExtendedCommandId::Box),
                CommandPayload::Points(vec![min, max]),
            ),
        )
        .unwrap_or_else(|error| panic!("BOX creates a body: {error:?}"));
    EntityId(outcome.objects[0].0)
}

fn revision_of(controller: &HostController) -> cad_domain::Revision {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .revision()
}

#[test]
fn union_of_disjoint_bodies_is_refused_as_multi_shell_and_writes_nothing() {
    let mut controller = modeler_controller();
    let document = controller.document_id;
    // Two boxes that cannot touch: their union is a multi-lump body.
    let first = create_box(&mut controller, p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0));
    let second = create_box(&mut controller, p(5.0, 0.0, 0.0), p(6.0, 1.0, 1.0));
    assert_eq!(solid_count(&controller), 2, "two created bodies");
    controller.session.selection =
        SelectionSet::from_refs([first, second].iter().map(|id| SelectionRef {
            document,
            entity: *id,
            instance: InstancePath::default(),
            sub_element: None,
        }));

    let before_revision = revision_of(&controller);
    let before_undo = controller.application.history[&document].undo_depth();
    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                controller.viewport_id,
                CommandId::Extended(ExtendedCommandId::Union),
                CommandPayload::None,
            ),
        )
        .expect("UNION starts the background job");
    assert!(outcome.objects.is_empty(), "the command writes nothing yet");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "solid.boolean_started"),
        "the job start is reported"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let error = loop {
        match controller.poll_solid_boolean() {
            SolidBooleanPoll::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the real disjoint union did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            SolidBooleanPoll::Failed { error } => break error,
            other => panic!("a disjoint union must be refused, got {other:?}"),
        }
    };
    let reason = match error {
        cad_domain::CadError::Unsupported(reason) => reason,
        other => panic!("a representability refusal is Unsupported, got {other:?}"),
    };
    assert!(
        reason.starts_with("solid.boolean_result_not_single_shell:"),
        "{reason}"
    );

    assert_eq!(solid_count(&controller), 2, "both operands survive");
    assert_eq!(revision_of(&controller), before_revision, "no write");
    assert_eq!(
        controller.application.history[&document].undo_depth(),
        before_undo,
        "no undo record"
    );
    for id in [first, second] {
        assert!(
            controller.application.workspace.documents[&document]
                .drawing
                .entity(id)
                .is_some(),
            "operand {} still exists",
            id.0
        );
    }
}
