//! Command normalization, status projection and lifecycle session tests.

use super::super::input::{interaction_request, scroll_delta, POINTER_ID};
use super::super::*;
use super::test_support::*;

/// The exhaustive `CommandId` → canonical-selector contract for
/// [`COMMAND_TABLE`].
///
/// The generated `canonical_selector` `match` has **no wildcard arm**, so a
/// new `cad_app::CommandId` variant fails to compile until it is listed here.
/// `ALL_COMMAND_IDS`/`EXPECTED_COMMAND_COUNT` come from the same list, so the
/// test below can require every variant to appear in the table exactly once:
/// "44/44" is enforced by the type checker and the test, not by prose.
macro_rules! command_selector_contract {
        ($($variant:ident => $selector:literal),+ $(,)?) => {
            const EXPECTED_COMMAND_COUNT: usize = [$(stringify!($variant)),+].len();
            const ALL_COMMAND_IDS: &[CommandId] = &[$(CommandId::$variant),+];

            fn canonical_selector(id: CommandId) -> &'static str {
                match id {
                    $(CommandId::$variant => $selector,)+
                }
            }
        };
    }

command_selector_contract! {
    OpenDrawing => "OPEN",
    NewDrawing => "NEW",
    PlotDrawing => "PLOT",
    SaveDrawingAs => "SAVE",
    CancelLoading => "CANCEL",
    Pan => "PAN",
    Zoom => "ZOOM",
    FitDrawing => "FIT",
    ResetView => "RESETVIEW",
    ToggleLayer => "TOGGLELAYER",
    SetLayerVisibilities => "SETLAYERS",
    RestoreLayers => "RESTORELAYERS",
    SwitchSpace => "SPACE",
    Select => "SELECT",
    SelectAll => "SELECTALL",
    SelectInBox => "SELECTINBOX",
    Measure => "MEASURE",
    ConfirmMeasurement => "CONFIRM",
    CancelMeasurement => "CANCELMEASURE",
    Undo => "UNDO",
    Redo => "REDO",
    Resources => "RESOURCES",
    Diagnostics => "DIAGNOSTICS",
    SwitchBackend => "SWITCHBACKEND",
    Switch2d3d => "SWITCH2D3D",
    StandardView => "STANDARDVIEW",
    Orbit => "ORBIT",
    SwitchProjection => "PROJECTION",
    SetMode => "MODE",
    CreateLine => "LINE",
    CreateCircle => "CIRCLE",
    CreateArc => "ARC",
    MoveEntities => "MOVE",
    TrimEntity => "TRIM",
    ExtendEntity => "EXTEND",
    EraseEntities => "ERASE",
    CopyEntities => "COPY",
    RotateEntities => "ROTATE",
    ScaleEntities => "SCALE",
    MirrorEntities => "MIRROR",
    CopyToClipboard => "COPYCLIP",
    CutToClipboard => "CUTCLIP",
    PasteFromClipboard => "PASTECLIP",
    OffsetEntity => "OFFSET",
    SetActiveLayer => "SETACTIVELAYER",
    ToggleHoverPreview => "HOVERPREVIEW",
    ToggleSnap => "OSNAP",
}

/// A committed synthetic, writer-produced AC1032 DWG with four LINE
/// entities (`fixtures/manifest`). It is not a vendor sample.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../../../fixtures/dwg/synthetic-four-lines.dwg");

#[test]
fn command_normalization_covers_every_variant() {
    assert_eq!(normalize_command(" open "), Some(CommandId::OpenDrawing));
    assert_eq!(normalize_command("FIT"), Some(CommandId::FitDrawing));
    assert_eq!(normalize_command("fit\n"), Some(CommandId::FitDrawing));
    assert_eq!(normalize_command("line"), Some(CommandId::CreateLine));
    assert_eq!(normalize_command("export"), Some(CommandId::PlotDrawing));
    // Contract: `canonical_selector` is an exhaustive `match` over
    // `CommandId` with no wildcard arm, so a new variant cannot be added
    // without a selector here. Each variant must then appear in
    // `COMMAND_TABLE` exactly once (no missing row, no duplicate mapping).
    assert_eq!(
        EXPECTED_COMMAND_COUNT, 47,
        "the cad_app CommandId set changed; update COMMAND_TABLE and this contract"
    );
    assert_eq!(ALL_COMMAND_IDS.len(), EXPECTED_COMMAND_COUNT);
    for id in ALL_COMMAND_IDS {
        let name = canonical_selector(*id);
        assert_eq!(normalize_command(name), Some(*id), "{name}");
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
        assert_eq!(
            COMMAND_TABLE
                .iter()
                .filter(|(_, mapped)| *mapped == *id)
                .count(),
            1,
            "{name} ({id:?}) must appear exactly once in COMMAND_TABLE"
        );
    }
    assert_eq!(COMMAND_TABLE.len(), EXPECTED_COMMAND_COUNT);
    // Every canonical selector maps to its command and is stable.
    for (name, id) in COMMAND_TABLE {
        assert_eq!(normalize_command(name), Some(*id), "{name}");
        assert_eq!(canonical_command_name(name), Some(*name), "{name}");
    }
    for rejected in ["", "BOGUS", "open fit", "OPENX"] {
        assert_eq!(normalize_command(rejected), None, "{rejected:?}");
        assert_eq!(canonical_command_name(rejected), None, "{rejected:?}");
    }
}

#[test]
fn unknown_command_is_refused_before_any_host_change() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let before = session.snapshot(None, None, 0);
    let error = session.command("BOGUS").unwrap_err();
    assert!(error.contains("BOGUS"), "{error}");
    assert_eq!(session.snapshot(None, None, 0), before);
}

#[test]
fn payload_requiring_command_is_refused_explicitly_not_silently() {
    // A mapped command whose variant needs a payload still goes through the
    // funnel; the application layer refuses it explicitly (never a silent
    // empty success). `LINE` now starts the interactive tool, so this contract
    // is asserted with a command that is still payload-requiring.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("STANDARDVIEW").unwrap_err();
    assert!(!error.is_empty(), "an explicit refusal, not empty success");
}

#[test]
fn dom_wheel_sign_zooms_in_on_wheel_up() {
    // DOM wheel-up is a negative deltaY; the host zooms in on a positive dy.
    assert_eq!(scroll_delta(3.0, -100.0), [3.0, 100.0]);
    assert_eq!(scroll_delta(0.0, 100.0), [0.0, -100.0]);
    // A positive host delta yields a > 1 zoom factor (the host's formula).
    assert!(1.0 + scroll_delta(0.0, -100.0)[1] * 0.0015 > 1.0);
}

#[test]
fn unsupported_and_malformed_inputs_are_explicit() {
    let resize = probe(InputKind::Resize);
    assert!(interaction_request(&resize, 0).is_err());

    let mut missing = probe(InputKind::PointerDown);
    missing.width = Some(100.0);
    assert!(interaction_request(&missing, 0).is_err());

    let mut non_finite = probe(InputKind::Wheel);
    non_finite.x = Some(1.0);
    non_finite.y = Some(1.0);
    non_finite.dy = Some(f32::NAN);
    assert!(interaction_request(&non_finite, 0).is_err());
}

#[test]
fn pointer_and_wheel_requests_carry_canvas_local_revision() {
    let mut down = probe(InputKind::PointerDown);
    down.x = Some(20.0);
    down.y = Some(30.0);
    match interaction_request(&down, 7).unwrap() {
        InteractionRequest::Pointer {
            phase: PointerPhase::Down,
            position:
                PointInput::CanvasLocal {
                    logical_position,
                    metrics_revision,
                },
            button: Some(PointerButton::Primary),
            ..
        } => {
            assert_eq!(logical_position, [20.0, 30.0]);
            assert_eq!(metrics_revision, 7);
        }
        other => panic!("unexpected request: {other:?}"),
    }

    // A cancel without coordinates is accepted with the origin placeholder.
    assert!(interaction_request(&probe(InputKind::PointerCancel), 7).is_ok());
    assert!(interaction_request(&probe(InputKind::Blur), 7).is_ok());

    let mut wheel = probe(InputKind::Wheel);
    wheel.x = Some(5.0);
    wheel.y = Some(6.0);
    wheel.dx = Some(2.0);
    wheel.dy = Some(-4.0);
    match interaction_request(&wheel, 7).unwrap() {
        InteractionRequest::Scroll { delta_logical, .. } => {
            assert_eq!(delta_logical, [2.0, 4.0]);
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
fn status_error_is_locale_neutral() {
    let failed = StatusMessage::Catalog {
        key: "command.failed".into(),
        args: vec![cad_engine::protocol::StatusArgument {
            key: "error".into(),
            value: "parse failed".into(),
        }],
    };
    assert_eq!(status_error(&failed).as_deref(), Some("parse failed"));
    let opened = StatusMessage::Catalog {
        key: "linux.opened".into(),
        args: Vec::new(),
    };
    assert_eq!(status_error(&opened), None);
    assert_eq!(
        status_error(&StatusMessage::ExternalText("sync failed".into())).as_deref(),
        Some("sync failed")
    );
}

#[test]
fn resource_diagnostics_are_technical_not_ui_text() {
    let diagnostic = RawDiagnostic {
        object: None,
        code: RESOURCE_MISSING_CODE.into(),
        message: String::new(),
        severity: None,
        completeness: None,
        parameters: vec![RawDiagnosticParameter::Key("logo.png".into())],
    };
    assert_eq!(
        resource_diagnostic_text(&diagnostic),
        "resource.missing logo.png"
    );
    let keys = ["a.png".to_string(), "b.png".to_string()];
    assert_eq!(
        resource_keys_error(RESOURCE_MISSING_CODE, keys.iter()),
        "resource.missing a.png b.png"
    );
}

#[test]
fn blank_status_reports_no_document_capabilities() {
    let status = assemble_status(
        false,
        "ignored".into(),
        None,
        false,
        true,
        None,
        Some(CadCamera {
            center: [1.0, 2.0],
            scale: 1.0,
        }),
        None,
        None,
        CadSelection::empty(),
        0,
        0,
        None,
        false,
        false,
    );
    assert_eq!(status.document_label, None);
    assert_eq!(status.entity_count, None);
    assert_eq!(status.camera, None, "a missing document has no camera");
    assert!(!status.can_fit);
    assert!(status.can_open);
    assert_eq!(status.import_count, 0);
    assert_eq!(status.frame_count, 0);
    assert!(
        !status.hover_preview,
        "the assembled status carries the flag it was given"
    );
    assert!(
        !status.snap_enabled,
        "the assembled status carries the object-snap flag it was given"
    );
    // The selection block is always carried, never omitted.
    assert_eq!(status.selection, CadSelection::empty());
}

#[test]
fn document_status_reports_counts_and_camera() {
    let status = assemble_status(
        true,
        "fixture.dwg".into(),
        Some(3),
        false,
        true,
        None,
        Some(CadCamera {
            center: [10.0, 20.0],
            scale: 2.0,
        }),
        Some("Vulkan".into()),
        Some("hardware".into()),
        CadSelection::empty(),
        1,
        10,
        None,
        true,
        true,
    );
    assert_eq!(status.document_label.as_deref(), Some("fixture.dwg"));
    assert_eq!(status.entity_count, Some(3));
    assert!(status.can_fit);
    assert_eq!(status.frame_count, 10);
    assert!(
        status.hover_preview,
        "the status projects the session's hover preview flag"
    );
    assert!(
        status.snap_enabled,
        "the status projects the session's object-snap flag"
    );
}

#[test]
fn object_snap_selector_toggles_the_projected_session_flag_only() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };
    assert!(
        session.snapshot(None, None, 0).snap_enabled,
        "object snapping defaults on and is projected"
    );

    // The canonical selector and its alias both flip the session flag.
    session.command("OSNAP").expect("OSNAP is admitted");
    assert!(!session.snapshot(None, None, 0).snap_enabled);
    session.command("snap").expect("the SNAP alias is admitted");
    assert!(session.snapshot(None, None, 0).snap_enabled);

    // Session state only: no database revision, no undo entry.
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
}

#[test]
fn import_progress_projects_only_a_live_import_and_never_invents_counts() {
    // A pre-tick running snapshot: no phase, no totals, no bytes — the UI
    // must render indeterminate rather than a fabricated 0/0.
    let running = CadImportProgress::from_snapshot(&cad_app::ImportProgressSnapshot::running())
        .expect("a running import projects a progress block");
    assert_eq!(running.phase, None);
    assert_eq!(running.entities_done, 0);
    assert_eq!(running.entities_total, None);
    assert_eq!(running.bytes, None);
    assert!(running.cancellable);

    // A terminal (opened) snapshot is never exposed: the affordance must not
    // persist after the document is published.
    let opened =
        cad_app::ImportProgressSnapshot::terminal(cad_app::ImportTerminal::Opened { entities: 3 });
    assert_eq!(CadImportProgress::from_snapshot(&opened), None);
}

#[test]
fn session_builds_and_reports_the_demo_document() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let status = session.snapshot(None, None, 0);
    assert!(status.document_label.is_some());
    assert!(status.entity_count.is_some());
    assert!(status.can_fit);
    assert!(!status.loading);
    assert_eq!(status.import_count, 0, "the demo document is not an import");
    assert!(status.camera.is_some(), "the demo view is a 2D plan view");
}

#[test]
fn resource_install_failure_is_propagated_not_ignored() {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(FailingResources),
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: false,
            plot: false,
            import_worker: false,
        },
    };
    let result = CadSession::new(options(), platform, RenderSessionHandle::default());
    assert!(matches!(result, Err(CadError::Unsupported(_))));
}

#[test]
fn open_admission_does_not_claim_a_completed_import() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    session.command("OPEN").expect("OPEN is admitted");
    // A pending picker keeps the old document and increments nothing.
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 0);
    assert!(status.document_label.is_some());
    assert!(status.entity_count.is_some());
}

#[test]
fn failed_open_keeps_the_old_document_and_zero_imports() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    session.command("OPEN").expect("OPEN is admitted");
    files.events.borrow_mut().push(FileEvent::Failed {
        purpose: cad_platform::FilePurpose::Open,
        reason: "read failed".into(),
    });
    session.tick().expect("tick");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 0, "a failed open is not an import");
    assert!(status.document_label.is_some(), "the old document remains");
    assert!(status.entity_count.is_some());
    assert_eq!(status.error.as_deref(), Some("read failed"));
}

#[test]
fn successful_synthetic_import_counts_once_and_reports_real_entities() {
    let files = Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    });
    let session = session(files.clone());
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("synthetic-four-lines.dwg"),
        bytes: Arc::from(SYNTHETIC_DWG.to_vec().into_boxed_slice()),
    });
    session.command("OPEN").expect("OPEN is admitted");
    session.tick().expect("tick");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 1, "one verified successful import");
    assert_eq!(
        status.entity_count,
        Some(4),
        "the synthetic DWG has 4 lines"
    );
    assert_eq!(
        status.document_label.as_deref(),
        Some("synthetic-four-lines.dwg")
    );
    assert!(status.camera.is_some());
    assert_eq!(status.error, None);
    // Navigation and repeated snapshots never re-count.
    let mut move_input = move_input(5.0, 6.0);
    move_input.sequence = 2;
    session.handle_input(&move_input).unwrap();
    assert_eq!(session.snapshot(None, None, 0).import_count, 1);
}

#[test]
fn repeated_navigation_does_not_leak_queue_quota() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // Well past the queue's normal (240) and absolute (256) capacity: if
    // completions were not consumed, admission would fail with QueueFull.
    for index in 0..600u64 {
        let mut input = move_input(index as f32, index as f32);
        input.sequence = index + 1;
        session
            .handle_input(&input)
            .unwrap_or_else(|error| panic!("navigation {index} refused: {error}"));
    }
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.import_count, 0, "navigation is not an import");
    assert_eq!(status.error, None, "successful navigation leaves no error");
}

#[test]
fn stale_context_completion_surfaces_as_an_error_not_empty_success() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let context = session.core.request_context();
    let request = EngineRequest::Interaction(InteractionRequest::Pointer {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        phase: PointerPhase::Move,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: [1.0, 2.0],
            metrics_revision: context.metrics_revision.unwrap_or(0),
        },
        modifiers: Modifiers::default(),
    });
    let receipt = session.core.submit(context, request).unwrap();
    session.track_receipt(receipt.id);
    // The canvas metrics revision advances before the queued request runs,
    // so the host grades it stale.
    session.set_canvas_metrics([640.0, 480.0], 1.0).unwrap();
    session.pump().unwrap();
    let outcome = session.drain_completions();
    assert!(
        outcome.is_err(),
        "a stale completion must not be an empty success"
    );
    assert!(session.snapshot(None, None, 0).error.is_some());
}

#[test]
fn canvas_metrics_accept_valid_and_refuse_degenerate_input() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session.set_canvas_metrics([800.0, 600.0], 1.0).unwrap();
    assert!(session.set_canvas_metrics([0.0, 600.0], 1.0).is_err());
    assert!(session.set_canvas_metrics([800.0, 600.0], 0.0).is_err());
}

#[test]
fn tick_tracks_the_live_surface_rect_so_later_inputs_anchor_correctly() {
    let surface = MutableSurface::new();
    surface.set_metrics(SurfaceMetrics {
        logical: [640.0, 480.0],
        scale: 1.0,
        safe_insets: [0.0; 4],
    });
    let session = session_with_surface(
        Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }),
        surface.clone(),
    );
    assert_eq!(canvas_logical_size(&session), [640.0, 480.0]);
    // The frontend reports a new CAD rect. Until the owner tick re-reads the
    // surface the viewport still holds the creation-time size: exactly the
    // stale state that made an off-center wheel zoom anchor off the cursor.
    surface.set_metrics(SurfaceMetrics {
        logical: [1024.0, 768.0],
        scale: 1.0,
        safe_insets: [0.0; 4],
    });
    assert_eq!(
        canvas_logical_size(&session),
        [640.0, 480.0],
        "stale viewport until the owner tick re-reads the surface"
    );
    session.tick().expect("tick");
    assert_eq!(
        canvas_logical_size(&session),
        [1024.0, 768.0],
        "the tick must re-apply the accepted surface rect to the viewport"
    );
}

#[test]
fn note_frame_is_the_actual_present_count() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    assert_eq!(session.frames_presented(), 0);
    session.note_frame(12);
    assert_eq!(session.frames_presented(), 12);
    assert_eq!(session.snapshot(None, None, 0).frame_count, 12);
    assert_eq!(session.snapshot(None, None, 99).frame_count, 99);
}

#[test]
fn stale_resource_guard_clears_caches_once_per_generation() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // No failure status: nothing to guard.
    session.guard_stale_resources();
    assert_eq!(session.resource_guard.get(), None);
    // A resource-install failure on the current generation is guarded once.
    session
        .core
        .set_catalog_status(RESOURCE_FAILED_STATUS, &[("error", "no resolver")])
        .unwrap();
    session.guard_stale_resources();
    let generation = session.core.request_context().document_generation;
    assert_eq!(session.resource_guard.get(), Some(generation));
    session.guard_stale_resources();
    assert_eq!(session.resource_guard.get(), Some(generation));
}

#[test]
fn startup_diagnostics_accessor_is_exposed() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let _ = session.startup_diagnostics();
}

#[test]
fn admissible_names_cover_the_table_plus_aliases_exactly_once() {
    let names = admissible_names();
    // Every canonical selector is advertised.
    for (canonical, _) in COMMAND_TABLE {
        assert!(
            names.contains(canonical),
            "{canonical} missing from the admissible list"
        );
    }
    // Every alias is advertised (an alias is a name the host admits).
    for (alias, canonical) in COMMAND_ALIASES {
        assert!(
            names.contains(alias),
            "{alias} missing from the admissible list"
        );
        assert!(names.contains(canonical), "{canonical} missing");
    }
    // No duplicates: a frontend may build a set from this list.
    let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "duplicate admissible name");
    // Every advertised name is actually admitted by the normalizer.
    for name in &names {
        assert!(
            canonical_command_name(name).is_some(),
            "{name} is advertised but does not normalize"
        );
    }
}
