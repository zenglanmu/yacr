//! Wave 9 report/refusal contracts: `UNITS`, `DIM`, the `TXTEXP` explosion and
//! the stable named refusals around them.
//!
//! `UNITS`, `DIM` and `SPACETRANS` are read-only reports; `TXTEXP` is a real 2D
//! edit (font outlines to polylines, one transaction / one undo step); the rest
//! are stable named refusals. A refusal never writes the drawing, bumps the
//! revision or records history, so every non-writing class stays Viewer-legal.

use super::*;
use crate::app_extended_w9_reports::text_explode::install_outline_fonts;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_representation::FontEngine;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

#[test]
fn units_reports_the_fixed_display_rule_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::Units).expect("UNITS is admitted");
    assert!(outcome.changes.is_none());
    let report = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "units.report")
        .expect("UNITS reports the display rule");
    assert!(report.message.contains("4 decimal places"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "units.dialog_not_implemented"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dim_lists_the_implemented_dimension_selectors_without_creating_one() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::DimRouter).expect("DIM is admitted");
    assert!(outcome.changes.is_none());
    let names: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|row| row.code == "dim.available")
        .map(|row| row.message.clone())
        .collect();
    assert!(names.iter().any(|row| row.contains("DIMLINEAR")));
    assert!(names.iter().any(|row| row.contains("QDIM")));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "dim.router_only"));
    assert_eq!(revision(&app), before);
}

const REFUSALS: [(ExtendedCommandId, &str); 2] = [
    (
        ExtendedCommandId::DimExport,
        "dimex.unsupported: dimension-style export files are not modeled",
    ),
    (
        ExtendedCommandId::DimImport,
        "dimim.unsupported: dimension-style import files are not modeled",
    ),
];

#[test]
fn wave9_report_refusals_answer_with_their_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ext).expect_err("a refusal never succeeds");
        match error {
            CadError::Unsupported(reason) => assert_eq!(reason, expected),
            other => panic!("{ext:?} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn wave9_reports_and_refusals_are_viewer_legal() {
    for ext in [ExtendedCommandId::Units, ExtendedCommandId::DimRouter] {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        execute(&mut app, &mut session, ext)
            .unwrap_or_else(|error| panic!("{ext:?} is Viewer-legal: {error:?}"));
    }
    for ext in [ExtendedCommandId::DimExport, ExtendedCommandId::DimImport] {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        let error = execute(&mut app, &mut session, ext)
            .expect_err("the feature is still missing in Viewer mode");
        match error {
            CadError::Unsupported(reason) => {
                assert!(reason.contains("unsupported"), "{ext:?}: {reason}")
            }
            other => panic!("{ext:?} must reach its named refusal, got {other:?}"),
        }
    }
}

#[test]
fn spacetrans_converts_a_model_distance_to_paper_for_the_active_viewport() {
    let (mut app, mut session) = application_with_layouts();
    session.active_space = SpaceId::Paper(LayoutId(1));
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SpaceTrans),
                CommandPayload::SpaceTrans(100.0),
            ),
        )
        .expect("SPACETRANS is admitted in a paper layout");
    let row = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "spacetrans.converted")
        .expect("the conversion is reported");
    assert!(
        row.message.contains("100 model unit(s) = 1 paper unit(s)"),
        "{}",
        row.message
    );
    assert!(row.message.contains("model→paper"), "{}", row.message);
    let factor = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "spacetrans.factor")
        .expect("the direction/factor is reported");
    assert!(factor.message.contains("0.01"), "{}", factor.message);
    assert!(
        factor.message.contains("100 model unit(s) per paper unit"),
        "{}",
        factor.message
    );
    assert_eq!(revision(&app), before, "SPACETRANS writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn spacetrans_refuses_model_space_and_an_unresolvable_viewport() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SpaceTrans),
                CommandPayload::SpaceTrans(10.0),
            ),
        )
        .expect_err("model space has no viewport conversion");
    assert!(
        error.to_string().contains("spacetrans.model_space"),
        "{error}"
    );

    let (mut app, mut session) = application_with_layouts();
    session.active_space = SpaceId::Paper(LayoutId(2));
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SpaceTrans),
                CommandPayload::SpaceTrans(10.0),
            ),
        )
        .expect_err("an unresolved viewport is refused by name");
    assert!(
        error
            .to_string()
            .contains("spacetrans.unsupported_viewport"),
        "{error}"
    );

    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SpaceTrans),
                CommandPayload::SpaceTrans(-1.0),
            ),
        )
        .expect_err("a non-positive distance is refused");
    assert!(
        error.to_string().contains("spacetrans.invalid_distance"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// TXTEXP
// ---------------------------------------------------------------------------

/// The repository's committed open font plus, when this checkout staged it, the
/// third-party `complex.shx` face, installed once per process into the TXTEXP
/// outline hook (the carrier's role in production). `complex.shx` is not
/// committed (`docs/fonts.md`), so it is read at run time; without it the SHX
/// case skips loudly instead of failing to compile. No fallback chain is set, so
/// a text whose font key has no face really is unavailable.
fn outline_fonts() {
    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    // An SHX shape font draws pen linework, not filled contours, so it exercises
    // the open-stroke arm of the same path.
    match super::font_fixture::shx_face("complex.shx") {
        Some(face) => {
            engine
                .register("complex.shx", Arc::from(face.as_slice()))
                .unwrap();
        }
        None => eprintln!("{}", super::font_fixture::missing_face_hint("complex.shx")),
    }
    // The first installation wins; every TXTEXP test installs the same engine.
    install_outline_fonts(Arc::new(engine));
}

/// Whether the installed hook really carries the SHX face the SHX case needs.
fn shx_font_available() -> bool {
    crate::app_extended_w9_reports::text_explode::outline_fonts()
        .is_some_and(|fonts| fonts.contains("complex.shx"))
}

fn text_entity(id: u128, content: &str, font: Option<&str>, type_key: &str) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(3),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: content.into(),
            position: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            style: StyleId(0),
            height: 2.0,
            rotation: 0.0,
            font: font.map(str::to_string),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
        },
        draw_order: id as i64,
    }
}

/// Replace the default document with one holding `entities` on the `WALLS` layer.
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn entities(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .into_iter()
        .cloned()
        .collect()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn txtexp_explodes_text_into_closed_outline_polylines_in_one_undo_step() {
    outline_fonts();
    let (mut app, mut session) =
        app_with(vec![text_entity(1, "I", Some("osifont.ttf"), "AcDbText")]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::TextExplode)
        .expect("TXTEXP is admitted");
    assert!(outcome.changes.is_some(), "TXTEXP writes one transaction");
    let rows = entities(&app);
    assert!(
        rows.iter()
            .all(|entity| !matches!(entity.geometry, SemanticGeometry::Text { .. })),
        "the text source is consumed"
    );
    assert!(!rows.is_empty(), "the outline produced geometry");
    for row in &rows {
        let SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } = &row.geometry
        else {
            panic!("TXTEXP produces polylines, got {:?}", row.geometry);
        };
        assert!(*closed, "an sfnt glyph contour is a closed polyline");
        assert!(bulges.is_empty());
        assert!(points.len() >= 3);
        assert!(
            points.first() != points.last(),
            "the repeated closing point is dropped so no zero-length segment exists"
        );
        assert_eq!(row.layer, LayerId(3), "the outline keeps the text's layer");
        assert_eq!(row.space, SpaceId::Model);
        assert_eq!(row.object.type_key, "AcDbPolyline");
    }
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "exactly one revision bump"
    );
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "txtexp.partial"));

    // One undo restores the text and removes every produced polyline.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    let rows = entities(&app);
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0].geometry, SemanticGeometry::Text { .. }));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn txtexp_reports_the_unwrapped_mtext_rule() {
    outline_fonts();
    let mut mtext = text_entity(1, "AB", Some("osifont.ttf"), "AcDbMText");
    if let SemanticGeometry::Text { wrap_width, .. } = &mut mtext.geometry {
        *wrap_width = Some(3.0);
    }
    let (mut app, mut session) = app_with(vec![mtext]);
    select(&mut session, &[1]);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::TextExplode)
        .expect("TXTEXP explodes MTEXT through the same outline call");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "txtexp.mtext_no_wrap"));
    assert!(entities(&app)
        .iter()
        .all(|row| matches!(row.geometry, SemanticGeometry::Polyline { .. })));
}

#[test]
fn txtexp_refusals_are_named_and_write_nothing() {
    outline_fonts();
    let cases: Vec<(&str, Vec<DbEntity>, Vec<u128>)> = vec![
        ("txtexp.selection", Vec::new(), Vec::new()),
        (
            "txtexp.not_text",
            vec![DbEntity {
                object: DbObject {
                    id: ObjectId(1),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(1),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                draw_order: 1,
            }],
            vec![1],
        ),
        (
            "txtexp.font_unavailable",
            vec![text_entity(1, "I", Some("missing.ttf"), "AcDbText")],
            vec![1],
        ),
    ];
    for (code, rows, selection) in cases {
        let (mut app, mut session) = app_with(rows);
        select(&mut session, &selection);
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ExtendedCommandId::TextExplode)
            .expect_err("every refusal fails the command");
        let message = match error {
            CadError::Unsupported(message) | CadError::InvalidInput(message) => message,
            other => panic!("{code}: expected a named refusal, got {other:?}"),
        };
        assert!(message.contains(code), "{code}: {message}");
        assert_eq!(revision(&app), before, "{code} writes nothing");
        assert!(!app.can_undo(&DocumentId(1)), "{code} records no undo");
    }
}

#[test]
fn txtexp_is_work_only() {
    // The catalogue flag still calls TXTEXP Viewer-legal (it used to be a named
    // refusal); the handler owns the refusal because the command now writes.
    let (mut app, mut session) = application_with_document();
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is a legal mode");
    let error = execute(&mut app, &mut session, ExtendedCommandId::TextExplode)
        .expect_err("Viewer cannot explode text");
    assert!(matches!(error, CadError::PermissionDenied));
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn txtexp_keeps_shx_pen_strokes_as_open_polylines() {
    outline_fonts();
    if !shx_font_available() {
        eprintln!(
            "txtexp_keeps_shx_pen_strokes_as_open_polylines: {}",
            super::font_fixture::missing_face_hint("complex.shx")
        );
        return;
    }
    let (mut app, mut session) =
        app_with(vec![text_entity(1, "HI", Some("complex.shx"), "AcDbText")]);
    select(&mut session, &[1]);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::TextExplode)
        .expect("an SHX face outlines into strokes");
    let rows: Vec<SemanticGeometry> = entities(&app)
        .into_iter()
        .map(|entity| entity.geometry)
        .collect();
    assert!(!rows.is_empty(), "the strokes produced geometry");
    let open = rows
        .iter()
        .filter(|geometry| matches!(geometry, SemanticGeometry::Polyline { closed: false, .. }))
        .count();
    assert!(open > 0, "SHX linework stays open: {rows:?}");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "txtexp.open_contours"),
        "the open-stroke divergence is reported, never hidden: {:?}",
        outcome.diagnostics
    );
}
