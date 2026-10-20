//! Capture-candidate tint contracts (TRIM/EXTEND).
//!
//! While an entity-pick step is pending the drawing tool owns the pointer, so
//! idle hover is suppressed. The entity under the cursor is published on
//! `DrawPreview::candidate`, and the host feeds it through the existing
//! `OverlayInputs::hovered` / `FLAG_HOVERED` channel. These tests drive the real
//! host request path (no UI, no GPU) against a controlled drawing.

use super::*;
use cad_app::{AppMode, DrawPreview};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};
use cad_domain::{
    Completeness, DatabaseId, EntityId, LayerId, LayoutId, ObjectId, Revision, SemanticGeometry,
    Transform3,
};

/// Controlled drawing for the capture-candidate tests.
///
/// Coordinates sit inside the default top view (`ortho scale = 1.0`, world
/// `x ∈ [-400, 400]`, `y ∈ [-300, 300]`) with hundreds of world units between
/// entities, far outside the 3-logical-pixel pickbox.
///
/// * `101` — LINE at `y = 100`, the TRIM target.
/// * `102` — LINE at `x = 0`, the boundary.
/// * `103` — CIRCLE at `(250, -200)`, a distinct third entity.
///
/// It also carries one paper layout so the `SwitchSpace` transition can be
/// exercised through the real command funnel.
fn install_trim_fixture(core: &HostCore) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(fixture_entity(101, line(-200.0, 100.0, 200.0, 100.0), 0))
        .unwrap();
    builder
        .insert_entity(fixture_entity(102, line(0.0, -100.0, 0.0, 300.0), 1))
        .unwrap();
    builder
        .insert_entity(fixture_entity(
            103,
            SemanticGeometry::Circle {
                center: p(250.0, -200.0),
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: 50.0,
            },
            2,
        ))
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: vec![PaperViewport {
                clip: vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)],
                model_to_paper: Transform3::scale(100.0),
                completeness: Completeness::Complete,
            }],
        })
        .unwrap();

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
    // These contracts are about the entity pick, not about snapping.
    controller.set_snap_enabled(false);
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p(x0, y0),
        end: p(x1, y1),
    }
}

fn fixture_entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn trim_core() -> HostCore {
    let core = hover_core();
    install_trim_fixture(&core);
    core
}

fn entity_ref(core: &HostCore, entity: u128) -> SelectionRef {
    let document = core.controller.borrow().document_id;
    cad_app::selection::model_ref(document, EntityId(entity))
}

/// Dispatch the canvas-local `MoveCursor` the session emits for a pointer move.
fn move_cursor(core: &HostCore, pixel: [f64; 2]) {
    dispatch_pointer(
        core,
        InteractionRequest::MoveCursor(PointInput::CanvasLocal {
            logical_position: pixel,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        }),
    );
}

/// Dispatch a click at a world point (the capture path resolves its own pick).
fn capture_world(core: &HostCore, point: Point3) {
    dispatch_pointer(
        core,
        InteractionRequest::CapturePoint(PointInput::World(point)),
    );
}

fn begin_trim(core: &HostCore) {
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Trim)
        .unwrap();
}

fn preview_candidate(core: &HostCore) -> Option<SelectionRef> {
    core.controller
        .borrow()
        .session
        .draw_preview()
        .expect("the Trim capture has a preview")
        .candidate
}

/// Whether the renderer's flags producer would emit a `highlight.unresolved`
/// for the current overlay inputs.
///
/// This runs the **real** `selection_entity_flags` derivation the renderer uses
/// over the published `OverlayInputs`: a `hovered`/`selection` ref the database
/// does not resolve becomes a `highlight.unresolved` "… is not present in the
/// document" diagnostic. A cleared/dropped ref must produce none.
fn has_unresolved_highlight(core: &HostCore) -> bool {
    let c = core.controller.borrow();
    let drawing = c.drawing().expect("the host always has a document");
    cad_app::render_scene::selection_entity_flags(&drawing, &core.render.overlays(), c.document_id)
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "highlight.unresolved")
}

#[test]
fn trim_capture_tints_the_candidate_and_clears_over_empty_space() {
    let core = trim_core();
    begin_trim(&core);
    let target = entity_ref(&core, 101);

    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert_eq!(
        hovered_refs(&core),
        vec![target.clone()],
        "the entity under the cursor must carry the FLAG_HOVERED tint"
    );
    assert_eq!(preview_candidate(&core), Some(target));
    // The idle hover producer never ran: during a capture the candidate is the
    // only source of the tint, so there is no second producer to flicker.
    assert_eq!(core.hover.queries.get(), 0, "no idle hover pick may run");
    assert!(core.hover.hovered.borrow().is_empty());

    let empty = screen_of(&core, p(350.0, 250.0));
    assert!(picks_nothing(&core, empty), "the corner really is empty");
    move_cursor(&core, empty);
    assert!(
        hovered_refs(&core).is_empty(),
        "a cursor over empty space must leave nothing tinted"
    );
    assert_eq!(preview_candidate(&core), None);
}

#[test]
fn candidate_follows_the_second_step_after_the_first_pick() {
    let core = trim_core();
    begin_trim(&core);
    let target = entity_ref(&core, 101);
    let boundary = entity_ref(&core, 102);

    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert_eq!(hovered_refs(&core), vec![target.clone()]);

    // Capturing the first step clears the candidate; the committed pick is now
    // the tint on its own, so the target stays visible while the boundary is
    // chosen (N1).
    capture_world(&core, p(-150.0, 100.0));
    assert_eq!(
        preview_candidate(&core),
        None,
        "a captured step must not keep its old candidate"
    );
    assert_eq!(
        hovered_refs(&core),
        vec![target.clone()],
        "the committed pick stays tinted after the candidate is cleared"
    );

    // The next move resolves the pending boundary step; the tint is the
    // committed target plus the boundary candidate, in pick-then-candidate
    // order.
    move_cursor(&core, screen_of(&core, p(0.0, 250.0)));
    assert_eq!(
        hovered_refs(&core),
        vec![target.clone(), boundary.clone()],
        "the committed pick and the boundary candidate are tinted together"
    );

    let preview = core.controller.borrow().session.draw_preview().unwrap();
    assert_eq!(preview.picks, vec![target]);
    assert_eq!(preview.remaining, 1);
    assert_eq!(preview.candidate, Some(boundary));
}

#[test]
fn cancel_and_confirm_both_clear_the_candidate_tint() {
    // Cancel: the tint under the cursor is gone immediately.
    let cancelled = trim_core();
    begin_trim(&cancelled);
    move_cursor(&cancelled, screen_of(&cancelled, p(-150.0, 100.0)));
    assert!(!hovered_refs(&cancelled).is_empty());
    dispatch_pointer(&cancelled, InteractionRequest::Cancel);
    assert!(
        hovered_refs(&cancelled).is_empty(),
        "cancel must clear the capture tint"
    );
    assert!(cancelled
        .controller
        .borrow()
        .session
        .draw_preview()
        .is_none());
    assert_eq!(cancelled.hover.queries.get(), 0);

    // Confirm: after both picks and a successful trim, nothing is tinted and
    // the idle pointer still produces no tint by itself.
    let confirmed = trim_core();
    begin_trim(&confirmed);
    move_cursor(&confirmed, screen_of(&confirmed, p(-150.0, 100.0)));
    capture_world(&confirmed, p(-150.0, 100.0));
    move_cursor(&confirmed, screen_of(&confirmed, p(0.0, 250.0)));
    assert_eq!(
        hovered_refs(&confirmed),
        vec![entity_ref(&confirmed, 101), entity_ref(&confirmed, 102)],
        "the committed target and the boundary candidate are tinted together"
    );
    capture_world(&confirmed, p(0.0, 250.0));
    assert_eq!(
        hovered_refs(&confirmed),
        vec![entity_ref(&confirmed, 101), entity_ref(&confirmed, 102)],
        "both committed picks stay tinted until confirm"
    );

    dispatch_pointer(&confirmed, InteractionRequest::Confirm);
    assert!(
        confirmed
            .controller
            .borrow()
            .session
            .draw_preview()
            .is_none(),
        "the trim really confirmed (the capture ended)"
    );
    assert!(
        hovered_refs(&confirmed).is_empty(),
        "confirm must not leave a capture tint behind"
    );
    assert_eq!(confirmed.hover.queries.get(), 0);
}

#[test]
fn a_space_mode_or_document_change_clears_the_candidate_tint() {
    // Space: switching spaces through the real command funnel invalidates the
    // candidate resolved in the previous space.
    let space = trim_core();
    begin_trim(&space);
    move_cursor(&space, screen_of(&space, p(-150.0, 100.0)));
    assert!(!hovered_refs(&space).is_empty());
    space
        .command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
        )
        .expect("the fixture has the paper layout");
    assert!(
        hovered_refs(&space).is_empty(),
        "a space change must clear the capture tint"
    );
    assert_eq!(preview_candidate(&space), None);

    // Mode: a mode switch cancels an unconfirmed capture, so no tint survives.
    let mode = trim_core();
    begin_trim(&mode);
    move_cursor(&mode, screen_of(&mode, p(-150.0, 100.0)));
    assert!(!hovered_refs(&mode).is_empty());
    mode.command(CommandId::SetMode, CommandPayload::Mode(AppMode::Viewer))
        .expect("the mode switch is admitted");
    assert!(
        hovered_refs(&mode).is_empty(),
        "a mode change must clear the capture tint"
    );
    assert!(mode.controller.borrow().session.draw_preview().is_none());

    // Document: replacing the authoritative document rebuilds the session, so
    // the capture (and its candidate) cannot reference the old drawing.
    let document = trim_core();
    begin_trim(&document);
    move_cursor(&document, screen_of(&document, p(-150.0, 100.0)));
    assert!(!hovered_refs(&document).is_empty());
    document
        .controller
        .borrow_mut()
        .new_blank_document()
        .expect("a blank document is installed");
    document.note_document_replaced();
    document
        .sync_view()
        .expect("the replacement is republished");
    assert!(
        hovered_refs(&document).is_empty(),
        "a document change must clear the capture tint"
    );
    assert!(document
        .controller
        .borrow()
        .session
        .draw_preview()
        .is_none());
}

#[test]
fn idle_hover_stays_suppressed_and_leaves_no_tint_after_the_capture_ends() {
    // Idle hover preview explicitly disabled: the capture candidate still owns
    // the tint (it is tool feedback, not idle hover), while the idle producer
    // runs no pick at all.
    let disabled = trim_core();
    disabled
        .controller
        .borrow_mut()
        .session
        .set_hover_preview(false);
    begin_trim(&disabled);
    move_cursor(&disabled, screen_of(&disabled, p(-150.0, 100.0)));
    assert_eq!(disabled.hover.queries.get(), 0, "no idle pick runs");
    assert_eq!(hovered_refs(&disabled), vec![entity_ref(&disabled, 101)]);
    dispatch_pointer(&disabled, InteractionRequest::Cancel);
    assert!(
        hovered_refs(&disabled).is_empty(),
        "after the capture ends nothing is tinted"
    );

    // With the preview on, an idle move after the capture ends resolves its own
    // hover through the normal path — but immediately after the capture ends no
    // tint exists until such a move happens.
    let enabled = trim_core();
    begin_trim(&enabled);
    move_cursor(&enabled, screen_of(&enabled, p(-150.0, 100.0)));
    dispatch_pointer(&enabled, InteractionRequest::Cancel);
    assert!(hovered_refs(&enabled).is_empty());
    assert_eq!(enabled.hover.queries.get(), 0);
}

#[test]
fn a_pointer_leave_clears_the_capture_candidate_but_preserves_the_capture() {
    let core = trim_core();
    begin_trim(&core);
    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert_eq!(hovered_refs(&core), vec![entity_ref(&core, 101)]);

    dispatch_pointer(&core, InteractionRequest::ClearCursor);
    assert!(
        hovered_refs(&core).is_empty(),
        "the departed cursor must not leave a capture tint painted"
    );
    assert_eq!(preview_candidate(&core), None);
    // Leaving the surface is not a cancel: the capture itself survives.
    assert!(core.controller.borrow().session.draw_preview().is_some());
}

#[test]
fn a_candidate_only_tint_is_flags_only_while_the_preview_cursor_is_visual() {
    // The candidate travels on the existing entity-flags input, so a candidate
    // change over an otherwise identical preview is a flags-only publication:
    // no re-prepare and no base-scene work. This mirrors the real move, where
    // the preview cursor also moves (a visual overlay input) — the tint itself
    // adds no separate re-prepare.
    let core = core();
    let reference = entity_ref(&core, 101);
    let base = OverlayInputs {
        draw_preview: Some(DrawPreview {
            kind: DrawToolKind::Trim,
            points: Vec::new(),
            picks: Vec::new(),
            cursor: Some(p(1.0, 2.0)),
            candidate: None,
            remaining: 2,
            selection_count: 0,
            ready: false,
            polygon_sides: None,
            awaiting_text: false,
        }),
        ..OverlayInputs::default()
    };
    core.render.set_overlays(base.clone());
    let input_after_base = core.render.input_revision();

    let mut tinted = base.clone();
    tinted.hovered = cad_app::SelectionSet::from_refs([reference.clone()]);
    tinted.draw_preview.as_mut().unwrap().candidate = Some(reference.clone());
    let _ = core.render.set_overlays(tinted);
    assert_eq!(
        core.render.input_revision(),
        input_after_base,
        "a candidate-only tint must stay on the flags path (no re-prepare)"
    );

    // The committed picks join the **same** `FLAG_HOVERED` set (N1), so a
    // pick-set change over an otherwise identical preview is still a flags-only
    // publication: `picks` and `candidate` are not overlay-batch inputs (only
    // `points`/`cursor` are), and the derived `hovered` set changes, so this
    // must not re-prepare either.
    let second = entity_ref(&core, 102);
    let mut picked = base.clone();
    {
        let preview = picked.draw_preview.as_mut().unwrap();
        preview.picks = vec![reference.clone()];
        preview.candidate = Some(second.clone());
    }
    picked.hovered = cad_app::SelectionSet::from_refs([reference.clone(), second.clone()]);
    let _ = core.render.set_overlays(picked);
    assert_eq!(
        core.render.input_revision(),
        input_after_base,
        "a picks+candidate tint must stay on the flags path (no re-prepare)"
    );

    // Moving the preview cursor (what the live tool does on every pointer move)
    // is a visual overlay input and does schedule the overlay prepare.
    let mut moved = base;
    moved.draw_preview.as_mut().unwrap().cursor = Some(p(3.0, 4.0));
    let _ = core.render.set_overlays(moved);
    assert!(
        core.render.input_revision() > input_after_base,
        "the preview cursor is visual overlay input, so a cursor move re-prepares"
    );
}

#[test]
fn the_committed_picks_and_the_pending_candidate_are_tinted_together() {
    let core = trim_core();
    begin_trim(&core);
    let target = entity_ref(&core, 101);
    let boundary = entity_ref(&core, 102);

    // Step 1 commits the target: with no candidate left the committed pick is
    // the tint on its own (AutoCAD shows the chosen object as it is picked).
    capture_world(&core, p(-150.0, 100.0));
    assert_eq!(preview_candidate(&core), None);
    assert_eq!(
        hovered_refs(&core),
        vec![target.clone()],
        "the first committed pick must be tinted"
    );

    // While the boundary is hovered, the committed target and the pending
    // boundary candidate are tinted together, in pick-then-candidate order.
    move_cursor(&core, screen_of(&core, p(0.0, 250.0)));
    assert_eq!(
        hovered_refs(&core),
        vec![target.clone(), boundary.clone()],
        "the committed pick and the boundary candidate share the tint"
    );

    // Second pick, then confirm: the capture ends and nothing is tinted.
    capture_world(&core, p(0.0, 250.0));
    dispatch_pointer(&core, InteractionRequest::Confirm);
    assert!(core.controller.borrow().session.draw_preview().is_none());
    assert!(
        hovered_refs(&core).is_empty(),
        "confirm must leave no capture tint behind"
    );

    // Cancel: after a first pick the tint is cleared immediately too.
    let cancelled = trim_core();
    begin_trim(&cancelled);
    capture_world(&cancelled, p(-150.0, 100.0));
    assert_eq!(hovered_refs(&cancelled), vec![entity_ref(&cancelled, 101)]);
    dispatch_pointer(&cancelled, InteractionRequest::Cancel);
    assert!(
        hovered_refs(&cancelled).is_empty(),
        "cancel clears the committed-pick tint"
    );
}

#[test]
fn a_mid_capture_erase_drops_the_tinted_candidate_without_a_highlight_unresolved() {
    let core = trim_core();
    begin_trim(&core);
    let target = entity_ref(&core, 101);

    // Hover the target so it is the live candidate (no pick committed yet).
    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert_eq!(hovered_refs(&core), vec![target.clone()]);

    // ERASE at the command line is not capture-modal: it replaces the document
    // while the TRIM capture is still active, leaving the candidate dead.
    select_entity(&core, EntityId(101));
    core.command(CommandId::EraseEntities, CommandPayload::None)
        .expect("the erase runs mid-capture");

    assert!(
        core.controller.borrow().session.draw_preview().is_some(),
        "the capture is not cancelled by an edit"
    );
    assert_eq!(
        preview_candidate(&core),
        None,
        "the dead candidate must be cleared"
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "the dead candidate must not keep a tint"
    );
    assert!(
        !has_unresolved_highlight(&core),
        "a cleared candidate must not produce a highlight.unresolved"
    );

    // The very next move does not resurrect it: the entity is gone, so the
    // re-resolved candidate is honestly `None` and no diagnostic appears.
    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert!(hovered_refs(&core).is_empty());
    assert_eq!(preview_candidate(&core), None);
    assert!(!has_unresolved_highlight(&core));
}

#[test]
fn erasing_a_picked_entity_mid_capture_drops_its_tint_and_confirm_refuses() {
    let core = trim_core();
    begin_trim(&core);
    let target = entity_ref(&core, 101);
    let boundary = entity_ref(&core, 102);

    // Commit both picks; both are tinted (N1).
    capture_world(&core, p(-150.0, 100.0));
    move_cursor(&core, screen_of(&core, p(0.0, 250.0)));
    capture_world(&core, p(0.0, 250.0));
    assert_eq!(hovered_refs(&core), vec![target.clone(), boundary.clone()]);

    // Erase the picked target while the capture is still live.
    select_entity(&core, EntityId(101));
    core.command(CommandId::EraseEntities, CommandPayload::None)
        .expect("the erase runs mid-capture");

    assert!(core.controller.borrow().session.draw_preview().is_some());
    assert_eq!(
        hovered_refs(&core),
        vec![boundary.clone()],
        "only the still-resolving pick is tinted; the vanished one is dropped"
    );
    assert!(
        !has_unresolved_highlight(&core),
        "a vanished pick must not become a highlight.unresolved tint"
    );

    // CONFIRM refuses honestly: the target no longer exists, so the capture is
    // not silently committed against a stale ref, and it is preserved.
    let refused = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Confirm),
    );
    assert!(refused.is_err(), "a vanished pick is refused at confirm");
    assert!(
        core.controller.borrow().session.draw_preview().is_some(),
        "a refused confirm preserves the capture"
    );
}

#[test]
fn a_mid_capture_edit_that_keeps_the_candidate_refreshes_the_tint() {
    let core = trim_core();
    begin_trim(&core);
    let target = entity_ref(&core, 101);

    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert_eq!(hovered_refs(&core), vec![target.clone()]);

    // Erase a *different* entity: the document content changes (a new scene
    // identity) but the candidate's entity survives.
    select_entity(&core, EntityId(103));
    core.command(CommandId::EraseEntities, CommandPayload::None)
        .expect("the erase runs mid-capture");

    assert_eq!(
        preview_candidate(&core),
        None,
        "the candidate is re-derived against the new document, never carried"
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "the pre-edit tint is dropped, never left stuck"
    );
    assert!(
        core.controller.borrow().session.draw_preview().is_some(),
        "the capture stays usable after an edit that kept the candidate"
    );

    // The next move re-resolves the same entity: the tint is simply refreshed.
    move_cursor(&core, screen_of(&core, p(-150.0, 100.0)));
    assert_eq!(hovered_refs(&core), vec![target.clone()]);
    assert_eq!(preview_candidate(&core), Some(target));
    assert!(!has_unresolved_highlight(&core));
}
