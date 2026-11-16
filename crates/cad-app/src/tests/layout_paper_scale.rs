//! `PSBSCALE`/`PSTSCALE` contracts for the **compound annotation** kinds:
//! `MULTILEADER` and `TABLE` text must scale by the same active-viewport
//! paper/model factor that the standalone `TEXT`/`MTEXT` path already applied.
//!
//! Pinned here:
//!
//! * both commands scale a `MULTILEADER`'s annotation text height (and the style
//!   snapshot's height with it) while leaving roots, lines and the plane frame
//!   untouched;
//! * both commands scale every `TABLE` cell height and the style snapshot height
//!   while leaving column widths, row heights and text untouched;
//! * one transaction / one undo step for the whole selection;
//! * the two domains stay disjoint and **named**: a standalone text entity under
//!   `PSBSCALE` is `psbscale.not_block`, a block reference under `PSTSCALE` is
//!   `pstscale.not_text`, a line under either is refused, and every refusal
//!   writes nothing.

use super::*;
use cad_db::{BlockDefinition, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};

const DOC: DocumentId = DocumentId(1);

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn command_with(id: ExtendedCommandId) -> Command {
    command(CommandId::Extended(id), CommandPayload::None)
}

fn rect_viewport(scale: f64) -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(0.0, 50.0),
        ],
        model_to_paper: Transform3::scale(scale),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

fn entity(
    id: u128,
    space: SpaceId,
    type_key: &str,
    geometry: SemanticGeometry,
) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: id as i64,
    }
}

/// One straight-line MULTILEADER with text content: height `height`, no width
/// boundary (the documented `0` sentinel).
fn leader(id: u128, space: SpaceId, height: f64) -> cad_db::DbEntity {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: height,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
            text_angle_type: MLeaderTextAngle::Horizontal,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "note".into(),
            location: p3(5.0, 5.0, 0.0),
            rotation: 0.0,
            height,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p3(5.0, 5.0, 0.0),
            dogleg_vector: p3(1.0, 0.0, 0.0),
            dogleg_length: 1.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    entity(
        id,
        space,
        "AcDbMultiLeader",
        SemanticGeometry::MultiLeader(Box::new(geometry)),
    )
}

/// A straight-line MULTILEADER whose annotation is a feature-control frame.
fn tolerance_leader(id: u128, space: SpaceId, height: f64) -> cad_db::DbEntity {
    let geometry = MultiLeaderGeometry::created_with_content(
        MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: height,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
            text_angle_type: MLeaderTextAngle::Horizontal,
        },
        "Standard".into(),
        2,
        p3(0.0, 0.0, 0.0),
        p3(5.0, 5.0, 0.0),
        cad_domain::MLeaderCreateContent::Tolerance {
            text: "|0.05".into(),
        },
        Default::default(),
    )
    .expect("a valid tolerance leader");
    entity(
        id,
        space,
        "AcDbMultiLeader",
        SemanticGeometry::MultiLeader(Box::new(geometry)),
    )
}

/// A straight-line MULTILEADER whose annotation is an inserted block.
fn block_leader(id: u128, space: SpaceId, height: f64) -> cad_db::DbEntity {
    let geometry = MultiLeaderGeometry::created_with_content(
        MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: height,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
            text_angle_type: MLeaderTextAngle::Horizontal,
        },
        "Standard".into(),
        2,
        p3(0.0, 0.0, 0.0),
        p3(5.0, 5.0, 0.0),
        cad_domain::MLeaderCreateContent::Block {
            block: BlockId(7),
            name: "Door".into(),
        },
        Default::default(),
    )
    .expect("a valid block leader");
    entity(
        id,
        space,
        "AcDbMultiLeader",
        SemanticGeometry::MultiLeader(Box::new(geometry)),
    )
}

fn cell(text: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: text.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: cad_domain::TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
        text_color: None,
        fill_color: None,
    }
}

/// A 1x2 table at the origin: 4 + 6 wide, 3 high, both cells 2.5 high.
fn table(id: u128, space: SpaceId) -> cad_db::DbEntity {
    let geometry = TableGeometry::new(TableParts {
        frame: TableFrame::default(),
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: "A".into(),
            },
            TableColumnGeometry {
                width: 6.0,
                name: "B".into(),
            },
        ],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![cell("id"), cell("name")],
        }],
        style_name: "Standard".into(),
        name: "Data".into(),
        description: String::new(),
    })
    .expect("the fixture table is valid");
    entity(
        id,
        space,
        "AcadTable",
        SemanticGeometry::Table(Box::new(geometry)),
    )
}

fn text(id: u128, space: SpaceId) -> cad_db::DbEntity {
    entity(
        id,
        space,
        "AcDbText",
        SemanticGeometry::Text {
            text: "note".into(),
            position: point(1.0, 1.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: TextLineSpacing::default(),
        },
    )
}

fn block(id: u128, space: SpaceId) -> cad_db::DbEntity {
    entity(
        id,
        space,
        "AcDbBlockReference",
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: Transform3::scale(1.0),
        },
    )
}

fn line(id: u128, space: SpaceId) -> cad_db::DbEntity {
    entity(
        id,
        space,
        "AcDbLine",
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    )
}

/// A drawing with layout 1 (`Sheet1`, one 1:100 viewport), a block definition so
/// an `INSERT` validates, and the given entities.
fn fixture(entities: Vec<cad_db::DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: cad_domain::BlockId(0),
            entities: Vec::new(),
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: vec![rect_viewport(100.0)],
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    let mut session = SessionState::new(DOC, AppMode::Work);
    session.active_space = SpaceId::Paper(LayoutId(1));
    (app, session)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DOC)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection.replace(ids.iter().map(|id| SelectionRef {
        document: DOC,
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn refusal(result: CadResult<CommandOutcome>) -> String {
    match result {
        Err(CadError::Unsupported(message)) | Err(CadError::InvalidInput(message)) => message,
        Err(other) => panic!("expected a named refusal, got {other:?}"),
        Ok(outcome) => panic!("expected a refusal, got {outcome:?}"),
    }
}

fn geometry(app: &Application, id: u128) -> SemanticGeometry {
    drawing(app)
        .entity(EntityId(id))
        .expect("the entity exists")
        .geometry
        .clone()
}

fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "{what}: expected {expected}, got {actual}"
    );
}

#[test]
fn pstscale_scales_the_multileader_content_payloads_too() {
    // A tolerance frame's cell height and a block content's placement scale are
    // annotation-scaled values: leaving them unscaled would make the stored
    // definition disagree with the scaled text height.
    let (mut app, mut session) = fixture(vec![
        tolerance_leader(30, SpaceId::Paper(LayoutId(1)), 0.2),
        block_leader(31, SpaceId::Paper(LayoutId(1)), 0.2),
    ]);
    select(&mut session, &[30, 31]);
    app.execute(&mut session, command_with(ExtendedCommandId::PstScale))
        .expect("PSTSCALE scales both content payloads");

    // 1:100 ⇒ 0.01 paper units per model unit.
    match geometry(&app, 30) {
        SemanticGeometry::MultiLeader(leader) => {
            let tolerance = leader.tolerance_content().expect("the frame payload");
            assert_near(tolerance.height, 0.002, "the frame cell height");
            assert_near(leader.text().height, 0.002, "the mirrored text height");
            assert_eq!(tolerance.text, "|0.05", "the frame string is untouched");
        }
        other => panic!("the entity stays a MULTILEADER, got {other:?}"),
    }
    match geometry(&app, 31) {
        SemanticGeometry::MultiLeader(leader) => {
            let block = leader.block_content().expect("the block payload");
            assert_near(block.scale.x, 0.01, "the block x scale");
            assert_near(block.scale.y, 0.01, "the block y scale");
            assert_near(block.scale.z, 0.01, "the block z scale");
            assert_eq!(block.name, "Door", "the block identity is untouched");
        }
        other => panic!("the entity stays a MULTILEADER, got {other:?}"),
    }
}

#[test]
fn pstscale_scales_multileader_and_table_text_by_the_viewport_factor() {
    let (mut app, mut session) = fixture(vec![
        leader(20, SpaceId::Paper(LayoutId(1)), 0.2),
        table(21, SpaceId::Paper(LayoutId(1))),
    ]);
    select(&mut session, &[20, 21]);
    let before = revision(&app);
    let depth = undo_depth(&app);

    let outcome = app
        .execute(&mut session, command_with(ExtendedCommandId::PstScale))
        .expect("PSTSCALE scales the multileader and the table text");
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec!["pstscale.applied"]
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");

    match geometry(&app, 20) {
        SemanticGeometry::MultiLeader(leader) => {
            // 1:100 ⇒ paper units per model unit = 0.01.
            assert_near(leader.text().height, 0.002, "the multileader text height");
            assert_near(
                leader.style().text_height,
                0.002,
                "the style snapshot's text height",
            );
            assert_eq!(
                leader.text().width,
                0.0,
                "the no-width sentinel stays 0, never scaled away"
            );
            assert_eq!(leader.roots().len(), 1, "the leader structure is untouched");
            assert_eq!(leader.lines().len(), 1);
            assert_eq!(leader.text().string, "note", "the string is untouched");
        }
        other => panic!("the entity stays a MULTILEADER, got {other:?}"),
    }

    match geometry(&app, 21) {
        SemanticGeometry::Table(table) => {
            for (row, cells) in table.rows().iter().enumerate() {
                for (column, cell) in cells.cells.iter().enumerate() {
                    assert_near(
                        cell.text_height,
                        0.025,
                        &format!("cell ({row}, {column}) text height"),
                    );
                    assert_eq!(cell.text, if column == 0 { "id" } else { "name" });
                }
            }
            assert_near(
                table.style().text_height,
                0.025,
                "the table style snapshot text height",
            );
            assert_eq!(table.columns()[0].width, 4.0, "column widths are untouched");
            assert_eq!(table.rows()[0].height, 3.0, "row heights are untouched");
        }
        other => panic!("the entity stays a TABLE, got {other:?}"),
    }

    // Undo restores every stored length.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo replays the scale");
    match geometry(&app, 20) {
        SemanticGeometry::MultiLeader(leader) => {
            assert_near(leader.text().height, 0.2, "undo restores the text height");
            assert_near(leader.style().text_height, 0.2, "…and the style snapshot");
        }
        other => panic!("the entity stays a MULTILEADER, got {other:?}"),
    }
    match geometry(&app, 21) {
        SemanticGeometry::Table(table) => {
            assert_near(
                table.rows()[0].cells[0].text_height,
                2.5,
                "…and cell heights",
            );
        }
        other => panic!("the entity stays a TABLE, got {other:?}"),
    }
}

#[test]
fn psbscale_scales_the_same_compound_annotation_text() {
    let (mut app, mut session) = fixture(vec![
        leader(20, SpaceId::Paper(LayoutId(1)), 0.2),
        table(21, SpaceId::Paper(LayoutId(1))),
        block(22, SpaceId::Paper(LayoutId(1))),
    ]);
    select(&mut session, &[20, 21, 22]);
    let outcome = app
        .execute(&mut session, command_with(ExtendedCommandId::PsbScale))
        .expect("PSBSCALE scales block references and annotation text");
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec!["psbscale.applied"]
    );
    match geometry(&app, 20) {
        SemanticGeometry::MultiLeader(leader) => {
            assert_near(leader.text().height, 0.002, "the multileader text height");
        }
        other => panic!("the entity stays a MULTILEADER, got {other:?}"),
    }
    match geometry(&app, 21) {
        SemanticGeometry::Table(table) => {
            assert_near(
                table.rows()[0].cells[0].text_height,
                0.025,
                "the cell height",
            );
        }
        other => panic!("the entity stays a TABLE, got {other:?}"),
    }
    match geometry(&app, 22) {
        SemanticGeometry::Insert { transform, .. } => {
            assert_near(transform.matrix[0][0], 0.01, "the block scale");
        }
        other => panic!("the entity stays an INSERT, got {other:?}"),
    }
}

#[test]
fn the_two_scale_domains_stay_named_and_disjoint() {
    let (mut app, mut session) = fixture(vec![
        text(30, SpaceId::Paper(LayoutId(1))),
        block(31, SpaceId::Paper(LayoutId(1))),
        line(32, SpaceId::Paper(LayoutId(1))),
    ]);
    let before = revision(&app);

    // A standalone TEXT belongs to PSTSCALE, not to PSBSCALE.
    select(&mut session, &[30]);
    let plain_text = refusal(app.execute(&mut session, command_with(ExtendedCommandId::PsbScale)));
    assert!(plain_text.contains("psbscale.not_block"), "{plain_text}");

    // A block reference belongs to PSBSCALE, not to PSTSCALE.
    select(&mut session, &[31]);
    let insert = refusal(app.execute(&mut session, command_with(ExtendedCommandId::PstScale)));
    assert!(insert.contains("pstscale.not_text"), "{insert}");

    // A LINE carries no scalable text under either name.
    select(&mut session, &[32]);
    let line_psb = refusal(app.execute(&mut session, command_with(ExtendedCommandId::PsbScale)));
    assert!(line_psb.contains("psbscale.not_block"), "{line_psb}");
    let line_pst = refusal(app.execute(&mut session, command_with(ExtendedCommandId::PstScale)));
    assert!(line_pst.contains("pstscale.not_text"), "{line_pst}");

    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&app), 0, "a refusal records no history");
}
