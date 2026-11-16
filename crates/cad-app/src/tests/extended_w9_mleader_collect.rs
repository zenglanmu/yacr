//! `MLEADERCOLLECT` heterogeneity contracts (promotion round).
//!
//! COLLECT is a user request to merge, so a differing style, `90` override mask
//! or plane frame is **accepted** — but nothing may change hands silently:
//!
//! * the per-line values DXF can carry (path type `170`, arrowhead size `40`,
//!   both behind the line's `93` override word) are written as per-line
//!   overrides, so a collected line keeps the appearance it had;
//! * the per-entity values with no per-line group (text height `CONTEXT_DATA
//!   41`, landing gap `145`, text style `343`) are adopted from the base leader
//!   and each is named in `mleadercollect.style_adopted`;
//! * a differing plane frame is adopted and named in
//!   `mleadercollect.plane_adopted`;
//! * stored `reserved` data still refuses by name and writes nothing.
//!
//! Every accepted merge is one transaction / one undo step, and a refusal bumps
//! no revision and records no history.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// The style snapshot every fixture starts from (the pinned `Standard` shape).
fn standard_style() -> MLeaderStyleValues {
    MLeaderStyleValues {
        path_type: MLeaderPathType::StraightLineSegments,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 1.0,
        landing_gap: 0.1,
        arrowhead_size: 0.5,
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: None,
        scale_factor: 1.0,
        text_angle_type: MLeaderTextAngle::Horizontal,
    }
}

/// The fixture of one MULTILEADER: one root, one straight line, text content.
struct LeaderSpec<'a> {
    id: u128,
    text: &'a str,
    text_position: Point3,
    arrow: Point3,
    dogleg: Point3,
    dogleg_length: f64,
    text_height: f64,
    style: MLeaderStyleValues,
    mask: u32,
    plane: PlaneFrame,
    reserved: MLeaderReserved,
}

impl<'a> LeaderSpec<'a> {
    fn new(id: u128, text: &'a str, text_position: Point3) -> Self {
        LeaderSpec {
            id,
            text,
            text_position,
            arrow: p3(0.0, 5.0, 0.0),
            dogleg: p3(9.0, 5.0, 0.0),
            dogleg_length: 1.0,
            text_height: 0.2,
            style: standard_style(),
            mask: 0,
            plane: PlaneFrame::default(),
            reserved: MLeaderReserved::default(),
        }
    }

    fn build(self) -> DbEntity {
        let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
            style: self.style.clone(),
            style_name: "Standard".into(),
            override_mask: self.mask,
            content: MLeaderContent::Text,
            text: MLeaderText {
                string: self.text.into(),
                location: self.text_position,
                rotation: 0.0,
                height: self.text_height,
                width: 0.0,
                attachment_point: 2,
                left_attachment: 2,
            },
            roots: vec![MLeaderRoot {
                leader_index: 0,
                landing_point: self.text_position,
                dogleg_vector: p3(1.0, 0.0, 0.0),
                dogleg_length: self.dogleg_length,
            }],
            lines: vec![MLeaderLine {
                root_index: 0,
                index: 0,
                points: vec![self.arrow, self.dogleg],
                path_type: self.style.path_type,
                override_flags: 0,
                arrowhead_size: self.style.arrowhead_size,
                arrowhead_handle: None,
            }],
            reserved: self.reserved,
            plane: self.plane,
        });
        DbEntity {
            object: DbObject {
                id: ObjectId(self.id),
                type_key: "AcDbMultiLeader".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(self.id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::MultiLeader(Box::new(geometry)),
            draw_order: self.id as i64,
        }
    }
}

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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_entities(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .into_iter()
        .cloned()
        .collect()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn collect(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::Extended(ExtendedCommandId::MLeaderCollect),
            CommandPayload::None,
        ),
    )
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn leader_of(entity: &DbEntity) -> &MultiLeaderGeometry {
    match &entity.geometry {
        SemanticGeometry::MultiLeader(leader) => leader,
        other => panic!("expected a MULTILEADER, got {other:?}"),
    }
}

/// The source leader that differs from the base in every way the command now
/// accepts: spline path, bigger arrowhead, taller text, an override mask, a
/// different dogleg length and a non-default plane frame.
fn heterogeneous_source() -> DbEntity {
    let mut style = standard_style();
    style.path_type = MLeaderPathType::Spline;
    style.arrowhead_size = 2.0;
    style.landing_distance = 3.0;
    let mut spec = LeaderSpec::new(2, "B", p3(20.0, 0.0, 0.0));
    spec.arrow = p3(12.0, 0.0, 0.0);
    spec.dogleg = p3(19.0, 0.0, 0.0);
    spec.dogleg_length = 3.0;
    spec.text_height = 0.9;
    spec.style = style;
    spec.mask = 0x0000_0011;
    spec.plane = PlaneFrame {
        origin: p3(0.0, 0.0, 2.0),
        u: p3(0.0, 1.0, 0.0),
        v: p3(-1.0, 0.0, 0.0),
        normal_reversed: true,
    };
    spec.build()
}

#[test]
fn collect_carries_per_line_path_and_arrowhead_differences_as_overrides() {
    let (mut app, mut session) = app_with(vec![
        LeaderSpec::new(1, "A", p3(10.0, 5.0, 0.0)).build(),
        heterogeneous_source(),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = collect(&mut app, &mut session).expect("a heterogeneous selection collects");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(outcome.objects.len(), 1);

    let entities = model_entities(&app);
    assert_eq!(entities.len(), 1, "both sources are consumed");
    let leader = leader_of(&entities[0]);
    assert_eq!(leader.lines().len(), 2);

    // The base line keeps the entity-level values: no override word.
    assert_eq!(leader.lines()[0].override_flags, 0);
    // The collected line keeps its own path type and arrowhead size, which DXF
    // can carry per line (170/40 behind word 93).
    let collected = &leader.lines()[1];
    assert_eq!(
        collected.override_flags & MLeaderLine::PATH_TYPE,
        MLeaderLine::PATH_TYPE
    );
    assert_eq!(collected.path_type, MLeaderPathType::Spline);
    assert_eq!(
        collected.override_flags & MLeaderLine::ARROWHEAD_SIZE,
        MLeaderLine::ARROWHEAD_SIZE
    );
    assert!(approx(collected.arrowhead_size, 2.0));

    // The per-root dogleg length travelled with the root (LEADER 40), so it is
    // not an adopted value and needs no report.
    assert!(approx(leader.roots()[1].dogleg_length, 3.0));

    let adopted = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "mleadercollect.style_adopted")
        .expect("the adopted snapshot fields are reported");
    assert!(
        adopted.message.contains("annotation text height"),
        "{adopted:?}"
    );
    assert!(
        adopted.message.contains("per-line overrides"),
        "{adopted:?}"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "mleadercollect.representation"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn collect_adopts_the_base_text_height_mask_and_plane_and_names_them() {
    let (mut app, mut session) = app_with(vec![
        LeaderSpec::new(1, "A", p3(10.0, 5.0, 0.0)).build(),
        heterogeneous_source(),
    ]);
    select(&mut session, &[1, 2]);
    let outcome = collect(&mut app, &mut session).expect("accepted");
    let entities = model_entities(&app);
    let leader = leader_of(&entities[0]);

    // One annotation: the base's height, style and mask win.
    assert!(approx(leader.text().height, 0.2));
    assert_eq!(leader.text().string, "A\\PB");
    assert_eq!(leader.override_mask(), 0);
    assert_eq!(leader.plane(), PlaneFrame::default());

    let plane = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "mleadercollect.plane_adopted")
        .expect("the adopted plane frame is reported");
    assert!(plane.message.contains("entity 2"), "{plane:?}");
    assert!(
        plane.message.contains("world coordinates unchanged"),
        "{plane:?}"
    );

    // The vertices did not move: only the frame metadata is the base one's.
    assert!(approx(leader.lines()[1].points[0].x, 12.0));
    assert!(approx(leader.lines()[1].points[1].x, 19.0));
}

#[test]
fn collect_reports_no_adoption_for_matching_leaders() {
    let (mut app, mut session) = app_with(vec![
        LeaderSpec::new(1, "A", p3(10.0, 5.0, 0.0)).build(),
        LeaderSpec::new(2, "B", p3(20.0, 0.0, 0.0)).build(),
    ]);
    select(&mut session, &[1, 2]);
    let outcome = collect(&mut app, &mut session).expect("accepted");
    assert!(
        !outcome
            .diagnostics
            .iter()
            .any(|row| row.code.ends_with("_adopted")),
        "a homogeneous selection adopts nothing: {:?}",
        outcome.diagnostics
    );
    let entities = model_entities(&app);
    let leader = leader_of(&entities[0]);
    assert!(leader.lines().iter().all(|line| line.override_flags == 0));
}

#[test]
fn collect_still_refuses_stored_break_data_and_writes_nothing() {
    let mut reserved = MLeaderReserved::default();
    reserved.roots.push(MLeaderRootExtras {
        root_index: 0,
        content_valid: true,
        unknown: true,
        text_attachment_direction: 0,
        breaks: vec![MLeaderBreak {
            start: p3(1.0, 1.0, 0.0),
            end: p3(2.0, 2.0, 0.0),
        }],
    });
    let mut source = LeaderSpec::new(2, "B", p3(20.0, 0.0, 0.0));
    source.reserved = reserved;
    let (mut app, mut session) = app_with(vec![
        LeaderSpec::new(1, "A", p3(10.0, 5.0, 0.0)).build(),
        source.build(),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let error = collect(&mut app, &mut session).expect_err("stored break data is not dropped");
    assert!(
        error.to_string().contains("mleadercollect.reserved"),
        "{error}"
    );
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&app), 0, "a refusal records no history");
}
