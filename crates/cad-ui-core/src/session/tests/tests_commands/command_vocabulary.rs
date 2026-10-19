//! Command vocabulary contracts: normalization, the selector tables and their
//! exhaustiveness/disjointness invariants.

use super::super::super::*;
use super::super::test_support::*;

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
    CreatePolyline => "PLINE",
    CreateRectangle => "RECTANG",
    CreateEllipse => "ELLIPSE",
    CreatePolygon => "POLYGON",
    CreatePoint => "POINT",
    CreateSpline => "SPLINE",
    DimLinear => "DIMLINEAR",
    DimAligned => "DIMALIGNED",
    MoveEntities => "MOVE",
    TrimEntity => "TRIM",
    ExtendEntity => "EXTEND",
    ChamferLines => "CHAMFER",
    FilletLines => "FILLET",
    ArrayRectangular => "ARRAYRECT",
    ArrayPolar => "ARRAYPOLAR",
    AlignEntities => "ALIGN",
    StretchEntities => "STRETCH",
    EraseEntities => "ERASE",
    ExplodeEntities => "EXPLODE",
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
        EXPECTED_COMMAND_COUNT, 62,
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
    for rejected in ["", "BOGUS", "open fit", "OPENX", "SNAP"] {
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

    // The canonical selector flips the session flag. `SNAP` is AutoCAD's grid
    // snap (F9), a different feature this build does not implement, so it must
    // NOT be admitted as an object-snap selector.
    assert_eq!(normalize_command("SNAP"), None);
    assert_eq!(canonical_command_name("SNAP"), None);
    session.command("OSNAP").expect("OSNAP is admitted");
    assert!(!session.snapshot(None, None, 0).snap_enabled);
    assert!(
        session.command("snap").is_err(),
        "SNAP must not be accepted as an object-snap selector"
    );
    assert!(
        !session.snapshot(None, None, 0).snap_enabled,
        "a refused SNAP must leave the object-snap flag unchanged"
    );

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
fn reserved_drawing_names_are_not_measurement_selectors() {
    // `SNAP` must stay absent (grid snap ≠ object snap). `ANGLE` is not an
    // AutoCAD command at all (the real ones are `DIMANGULAR`/`MEASUREGEOM`), so
    // it must not shadow anything. `PLINE` is now the polyline **draw** selector
    // (`COMMAND_TABLE` → `BeginDraw(Polyline)`), never a measurement selector;
    // the explicit measurement selectors are `MEASUREPOLYLINE` / `MEASUREANGLE`.
    for reserved in ["SNAP", "ANGLE"] {
        assert_eq!(canonical_command_name(reserved), None, "{reserved}");
        assert_eq!(normalize_command(reserved), None, "{reserved}");
        assert!(!admissible_names().contains(&reserved), "{reserved}");
    }
    assert_eq!(normalize_command("PLINE"), Some(CommandId::CreatePolyline));
    assert_eq!(measurement_selector_kind("PLINE"), None);
    assert!(admissible_names().contains(&"PLINE"));
    // A reserved name is refused before the host, leaving the tool untouched.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    assert!(session.command("SNAP").is_err());
    assert!(session.command("ANGLE").is_err());
    assert_eq!(session.snapshot(None, None, 0).tool, None);
    for selector in ["MEASUREPOLYLINE", "MEASUREANGLE"] {
        assert!(
            admissible_names().contains(&selector),
            "{selector} must be advertised"
        );
        assert_eq!(canonical_command_name(selector), Some(selector));
    }
}

#[test]
fn admissible_names_cover_the_vocabulary_exactly_once() {
    let names = admissible_names();
    // Every canonical `CommandId` selector is advertised.
    for (canonical, _) in COMMAND_TABLE {
        assert!(
            names.contains(canonical),
            "{canonical} missing from the admissible list"
        );
    }
    // Every interaction-only selector (measurement kinds + zoom steps) is
    // advertised, so the frontend enables them without a second list.
    for (selector, _) in MEASUREMENT_SELECTORS {
        assert!(
            names.contains(selector),
            "{selector} missing from the admissible list"
        );
    }
    for (selector, _) in ZOOM_STEP_SELECTORS {
        assert!(
            names.contains(selector),
            "{selector} missing from the admissible list"
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

/// The measurement selector table is exhaustive over [`cad_app::MeasurementToolKind`]
/// and shares no name with any other selector family.
///
/// `MEASUREMENT_SELECTORS` is hand-written, so without this contract a fifth
/// `MeasurementToolKind` could silently have no selector and a measurement
/// button would start the wrong algorithm. `MeasurementToolKind::key` is an
/// exhaustive `match` in `cad-app` (no wildcard arm), so a new variant cannot
/// compile until it is listed there and in `ALL`; this test then fails until
/// `MEASUREMENT_SELECTORS` matches `ALL` one-to-one. The disjointness is a
/// **tested invariant**, not prose: `CadSession::command` resolves the
/// measurement selectors *before* `normalize_command`, so a name shared with
/// `COMMAND_TABLE`, an alias or `ZOOM_STEP_SELECTORS` would silently prefer the
/// measurement path. No such collision exists today.
#[test]
fn measurement_selectors_are_exhaustive_and_disjoint_from_the_command_vocabulary() {
    // No missing, no extra, no duplicate: one selector per kind, and each
    // selector resolves back to exactly its own kind.
    assert_eq!(
        MEASUREMENT_SELECTORS.len(),
        cad_app::MeasurementToolKind::ALL.len(),
        "MEASUREMENT_SELECTORS must cover MeasurementToolKind::ALL one-to-one"
    );
    for kind in cad_app::MeasurementToolKind::ALL {
        let selectors: Vec<&str> = MEASUREMENT_SELECTORS
            .iter()
            .filter(|(_, mapped)| *mapped == kind)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            selectors.len(),
            1,
            "{kind:?} must have exactly one selector: {selectors:?}"
        );
        assert_eq!(measurement_selector_kind(selectors[0]), Some(kind));
        assert_eq!(canonical_command_name(selectors[0]), Some(selectors[0]));
        assert_eq!(
            normalize_command(selectors[0]),
            None,
            "carries no CommandId"
        );
    }
    let unique: std::collections::BTreeSet<&str> = MEASUREMENT_SELECTORS
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        unique.len(),
        MEASUREMENT_SELECTORS.len(),
        "duplicate measurement selector name"
    );

    // Disjoint from every other selector family (the lookup-order invariant).
    for (name, _) in MEASUREMENT_SELECTORS {
        assert!(
            !COMMAND_TABLE.iter().any(|(candidate, _)| candidate == name),
            "{name} collides with COMMAND_TABLE (would silently prefer the measurement path)"
        );
        assert!(
            !COMMAND_ALIASES.iter().any(|(alias, _)| alias == name),
            "{name} collides with a COMMAND_ALIASES alias"
        );
        assert!(
            !COMMAND_ALIASES
                .iter()
                .any(|(_, canonical)| canonical == name),
            "{name} is an alias target and would shadow the measurement dispatch"
        );
        assert!(
            !ZOOM_STEP_SELECTORS
                .iter()
                .any(|(candidate, _)| candidate == name),
            "{name} collides with ZOOM_STEP_SELECTORS"
        );
    }
}
