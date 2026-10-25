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
    CreateText => "TEXT",
    MLeader => "MLEADER",
    TextEdit => "TEXTEDIT",
    DimLinear => "DIMLINEAR",
    DimAligned => "DIMALIGNED",
    DimRadius => "DIMRADIUS",
    DimDiameter => "DIMDIAMETER",
    DimCenter => "DIMCENTER",
    DimAngular => "DIMANGULAR",
    DimOrdinate => "DIMORDINATE",
    DimBaseline => "DIMBASELINE",
    DimContinue => "DIMCONTINUE",
    Hatch => "HATCH",
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
    SetEntityProperty => "SETENTITYPROPERTY",
    MatchProp => "MATCHPROP",
    ToggleHoverPreview => "HOVERPREVIEW",
    ToggleGrid => "GRID",
    ToggleSnapGrid => "SNAP",
    ToggleSnap => "OSNAP",
    SetSnapSpacing => "SNAPUNIT",
    ToggleOrtho => "ORTHO",
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
        EXPECTED_COMMAND_COUNT, 79,
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
    for rejected in ["", "BOGUS", "open fit", "OPENX", "SNAPX"] {
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
    // snap (F9), a different feature with its own selector, so it must NOT flip
    // the object-snap switch.
    assert_eq!(normalize_command("SNAP"), Some(CommandId::ToggleSnapGrid));
    session.command("OSNAP").expect("OSNAP is admitted");
    assert!(!session.snapshot(None, None, 0).snap_enabled);
    assert!(
        session.command("snap").is_ok(),
        "SNAP is admitted, but as the grid-snap selector"
    );
    assert!(
        !session.snapshot(None, None, 0).snap_enabled,
        "SNAP must not be accepted as an object-snap selector"
    );
    assert!(
        session.snapshot(None, None, 0).snap_grid_enabled,
        "SNAP flips its own grid-snap flag instead"
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
fn grid_selector_toggles_the_projected_display_flag_only() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };
    assert!(
        session.snapshot(None, None, 0).grid_enabled,
        "the display-grid switch defaults on and is projected"
    );

    // The canonical selector flips the session flag. `SNAP` is AutoCAD's grid
    // snap (F9), a different feature with its own selector, so it must NOT be
    // accepted as the display-grid selector.
    assert_eq!(normalize_command("SNAP"), Some(CommandId::ToggleSnapGrid));
    session.command("GRID").expect("GRID is admitted");
    assert!(!session.snapshot(None, None, 0).grid_enabled);
    assert!(
        session.command("snap").is_ok(),
        "SNAP is admitted, but as the grid-snap selector"
    );
    assert!(
        !session.snapshot(None, None, 0).grid_enabled,
        "SNAP must not be accepted as the display-grid selector"
    );
    assert!(
        session.snapshot(None, None, 0).snap_grid_enabled,
        "SNAP flips its own grid-snap flag instead"
    );

    // GRID and OSNAP are independent session switches.
    assert!(
        session.snapshot(None, None, 0).snap_enabled,
        "flipping the grid must not flip object snapping"
    );

    // Session state only: no database revision, no undo entry and no write to
    // the persisted `view.overlays.grid` preference (the second gate).
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
    assert!(
        !session.settings().grid,
        "GRID is session state; the persisted overlay preference is untouched"
    );
}

#[test]
fn ortho_selector_toggles_the_projected_drafting_flag_only() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };
    assert!(
        !session.snapshot(None, None, 0).ortho_enabled,
        "ORTHO defaults off (AutoCAD's initial state) and is projected"
    );
    // The effective config's interaction leaves are what a preference write would
    // change; the settings snapshot also projects live session state, so it can no
    // longer stand in for "nothing was persisted".
    let interaction_before = session.core.config_snapshot().effective.interaction;

    // The canonical selector flips the session flag. `SNAP` is AutoCAD's grid
    // snap (F9), a different feature with its own selector, so it must NOT be
    // accepted as the ORTHO selector either.
    assert_eq!(normalize_command("SNAP"), Some(CommandId::ToggleSnapGrid));
    session.command("ORTHO").expect("ORTHO is admitted");
    assert!(session.snapshot(None, None, 0).ortho_enabled);
    assert!(
        session.command("snap").is_ok(),
        "SNAP is admitted, but as the grid-snap selector"
    );
    assert!(
        session.snapshot(None, None, 0).ortho_enabled,
        "SNAP must not be accepted as the ORTHO selector"
    );
    assert!(
        session.snapshot(None, None, 0).snap_grid_enabled,
        "SNAP flips its own grid-snap flag instead"
    );

    // ORTHO is independent of the other drafting switches.
    let status = session.snapshot(None, None, 0);
    assert!(status.snap_enabled, "flipping ORTHO must not flip OSNAP");
    assert!(status.grid_enabled, "flipping ORTHO must not flip GRID");

    // Session state only: no database revision, no undo entry and no persisted
    // preference write (there is no ORTHO preference at all).
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
    assert_eq!(
        session.core.config_snapshot().effective.interaction,
        interaction_before,
        "ORTHO is session state; the selector writes no persisted preference"
    );

    // ...and it toggles back off with the same selector.
    session.command("ortho").expect("ORTHO is case-insensitive");
    assert!(!session.snapshot(None, None, 0).ortho_enabled);
}

#[test]
fn matchprop_is_admitted_with_its_autocad_alias() {
    // MATCHPROP is the selection-based property painter: the bare selector is
    // its whole input (`CommandPayload::None`), so it is admitted like any other
    // name-only command and advertised through `admissible_names()` — the list
    // the chrome reads instead of a hand-written mirror.
    assert_eq!(normalize_command("MATCHPROP"), Some(CommandId::MatchProp));
    assert_eq!(canonical_command_name("matchprop"), Some("MATCHPROP"));
    // AutoCAD's default abbreviation resolves onto the same `CommandId`.
    assert_eq!(normalize_command("MA"), Some(CommandId::MatchProp));
    assert_eq!(canonical_command_name(" ma "), Some("MATCHPROP"));
    assert!(admissible_names().contains(&"MATCHPROP"));
    assert!(admissible_names().contains(&"MA"));
    // It is neither a measurement selector nor a zoom step, so no earlier lookup
    // table can shadow it.
    assert_eq!(measurement_selector_kind("MATCHPROP"), None);
    assert_eq!(zoom_step_factor("MATCHPROP"), None);
    // Work-only like the other edits: Viewer refuses it at the command layer.
    assert!(CommandId::MatchProp.requires_work_mode());
}

#[test]
fn reserved_drawing_names_are_not_measurement_selectors() {
    // `SNAP` is a real command now (grid snap, F9), but it is **not** a
    // measurement selector and must not shadow one. `ANGLE` is not an AutoCAD
    // command at all (the real ones are `DIMANGULAR`/`MEASUREGEOM`), so it must
    // not shadow anything. `PLINE` is now the polyline **draw** selector
    // (`COMMAND_TABLE` → `BeginDraw(Polyline)`), never a measurement selector;
    // the explicit measurement selectors are `MEASUREPOLYLINE` / `MEASUREANGLE`.
    assert_eq!(normalize_command("SNAP"), Some(CommandId::ToggleSnapGrid));
    assert_eq!(measurement_selector_kind("SNAP"), None);
    assert_eq!(canonical_command_name("ANGLE"), None);
    assert_eq!(normalize_command("ANGLE"), None);
    assert!(!admissible_names().contains(&"ANGLE"));
    assert_eq!(normalize_command("PLINE"), Some(CommandId::CreatePolyline));
    assert_eq!(measurement_selector_kind("PLINE"), None);
    assert!(admissible_names().contains(&"PLINE"));
    // An unbound name is refused before the host, leaving the tool untouched.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
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
