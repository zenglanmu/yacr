//! `FOLLOWME` command contracts: profile/path partition, the swept mesh and
//! every named refusal.
//!
//! The command is kernel-free: it builds the mesh in the command layer, so these
//! tests check the produced triangles (closed, outward-facing, exact volume) and
//! the one-transaction insert directly, without a modeler.

use super::*;
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer};

const PROFILE: u128 = 950;
const PATH: u128 = 951;
const RING_A: u128 = 952;
const RING_B: u128 = 953;
const RING_C: u128 = 954;
const RING_D: u128 = 955;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

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

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn line(id: u128, start: Point3, end: Point3) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, id as i64)
}

fn polyline(id: u128, points: Vec<Point3>, closed: bool) -> DbEntity {
    let bulges = vec![0.0; points.len()];
    ent(
        id,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            width: 0.0,
        },
        id as i64,
    )
}

/// A 1 × 1 square in `z = 0` as one closed straight polyline.
fn square(id: u128) -> DbEntity {
    polyline(
        id,
        vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 0.0, 0.0),
            p3(1.0, 1.0, 0.0),
            p3(0.0, 1.0, 0.0),
        ],
        true,
    )
}

fn follow_me(
    app: &mut Application,
    session: &mut SessionState,
    entities: &[u128],
) -> CadResult<CommandOutcome> {
    let refs: Vec<SelectionRef> = entities.iter().map(|id| ref_of(*id)).collect();
    app.execute(
        session,
        command(
            CommandId::Extended(ExtendedCommandId::FollowMe),
            CommandPayload::FollowMeProfile(refs),
        ),
    )
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

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

/// The committed mesh of the one created row.
fn created_mesh(app: &Application, outcome: &CommandOutcome) -> Mesh {
    assert_eq!(outcome.objects.len(), 1, "one body is created");
    let id = outcome.objects[0];
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id.0))
        .expect("the created row exists");
    assert_eq!(entity.object.type_key, "AcDbSubDMesh");
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) => mesh.clone(),
        other => panic!("expected a mesh row, got {other:?}"),
    }
}

/// Signed volume of a closed triangle mesh (divergence theorem).
fn volume(mesh: &Mesh) -> f64 {
    mesh.triangles
        .iter()
        .map(|triangle| {
            let a = mesh.vertices[triangle[0] as usize];
            let b = mesh.vertices[triangle[1] as usize];
            let c = mesh.vertices[triangle[2] as usize];
            dot(a, cross(b, c)) / 6.0
        })
        .sum()
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// Every geometric edge is traversed once in each direction, so the mesh is
/// closed **and** consistently oriented (a duplicated vertex does not hide it).
fn assert_closed_and_oriented(mesh: &Mesh) {
    use std::collections::HashMap;
    type Vertex = (i64, i64, i64);
    let key = |p: Point3| -> Vertex {
        (
            (p.x * 1e9).round() as i64,
            (p.y * 1e9).round() as i64,
            (p.z * 1e9).round() as i64,
        )
    };
    let mut edges: HashMap<(Vertex, Vertex), i32> = HashMap::new();
    for triangle in &mesh.triangles {
        for pair in [[0, 1], [1, 2], [2, 0]] {
            let a = key(mesh.vertices[triangle[pair[0]] as usize]);
            let b = key(mesh.vertices[triangle[pair[1]] as usize]);
            *edges.entry((a, b)).or_insert(0) += 1;
        }
    }
    for ((a, b), count) in &edges {
        assert_eq!(
            *count, 1,
            "directed edge {a:?}->{b:?} appears {count} times"
        );
        assert_eq!(
            edges.get(&(*b, *a)).copied().unwrap_or(0),
            1,
            "edge {a:?}->{b:?} has no opposite twin, so the mesh is not closed"
        );
    }
}

#[test]
fn a_square_profile_sweeps_along_a_straight_path_into_one_mesh() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
    ]);
    let before = revision(&app);
    let outcome = follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    let mesh = created_mesh(&app, &outcome);
    // 1 × 1 over 4: exactly 4.
    assert!(
        (volume(&mesh) - 4.0).abs() < 1e-9,
        "volume {}",
        volume(&mesh)
    );
    assert_closed_and_oriented(&mesh);
    // Four walls × two triangles + two caps × two triangles.
    assert_eq!(mesh.triangles.len(), 12);
    assert!(mesh.face_sources.iter().all(|source| source
        .as_ref()
        .is_some_and(|source| source.source_key.starts_with("followme."))));
    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "followme.applied")
        .expect("the success is named");
    assert!(applied.message.contains("4"), "{}", applied.message);
}

#[test]
fn a_right_angle_path_produces_a_mitred_closed_mesh() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(
            PATH,
            vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 0.0, 3.0)],
            false,
        ),
    ]);
    let outcome = follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    let mesh = created_mesh(&app, &outcome);
    // 1 × 1 over 4 + 3: the mitre keeps the prism volume.
    assert!(
        (volume(&mesh) - 7.0).abs() < 1e-9,
        "volume {}",
        volume(&mesh)
    );
    assert_closed_and_oriented(&mesh);
    // Two segments × four walls + two caps, two triangles each.
    assert_eq!(mesh.triangles.len(), 20);
}

#[test]
fn a_line_ring_is_the_profile_and_an_open_polyline_is_the_path() {
    let (mut app, mut session) = app_with(vec![
        line(RING_A, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        line(RING_B, p3(1.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)),
        line(RING_C, p3(1.0, 1.0, 0.0), p3(0.0, 1.0, 0.0)),
        line(RING_D, p3(0.0, 1.0, 0.0), p3(0.0, 0.0, 0.0)),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 2.0)], false),
    ]);
    let outcome = follow_me(
        &mut app,
        &mut session,
        &[RING_A, RING_B, RING_C, RING_D, PATH],
    )
    .expect("a LINE ring and an open polyline path sweep");
    let mesh = created_mesh(&app, &outcome);
    assert!(
        (volume(&mesh) - 2.0).abs() < 1e-9,
        "volume {}",
        volume(&mesh)
    );
    assert_closed_and_oriented(&mesh);
}

#[test]
fn two_line_chains_are_refused_because_the_roles_cannot_be_told_apart() {
    // Both the ring and the path as LINE rows is genuinely ambiguous: the
    // command refuses instead of guessing which chain is which.
    let (mut app, mut session) = app_with(vec![
        line(RING_A, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        line(RING_B, p3(1.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)),
        line(RING_C, p3(1.0, 1.0, 0.0), p3(0.0, 1.0, 0.0)),
        line(RING_D, p3(0.0, 1.0, 0.0), p3(0.0, 0.0, 0.0)),
        line(PATH, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 2.0)),
    ]);
    let text = refusal_text(
        follow_me(
            &mut app,
            &mut session,
            &[RING_A, RING_B, RING_C, RING_D, PATH],
        )
        .unwrap_err(),
    );
    assert!(text.starts_with("followme.profile_missing:"), "{text}");
}

#[test]
fn a_concave_profile_is_capped_by_ear_clipping() {
    // An L-shaped profile of area 3, swept 2 along +Z: volume 6.
    let (mut app, mut session) = app_with(vec![
        polyline(
            PROFILE,
            vec![
                p3(0.0, 0.0, 0.0),
                p3(2.0, 0.0, 0.0),
                p3(2.0, 1.0, 0.0),
                p3(1.0, 1.0, 0.0),
                p3(1.0, 2.0, 0.0),
                p3(0.0, 2.0, 0.0),
            ],
            true,
        ),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 2.0)], false),
    ]);
    let outcome = follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    let mesh = created_mesh(&app, &outcome);
    assert!(
        (volume(&mesh) - 6.0).abs() < 1e-9,
        "volume {}",
        volume(&mesh)
    );
    assert_closed_and_oriented(&mesh);
}

#[test]
fn the_sweep_is_not_translated_by_the_session_elevation() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
    ]);
    session.drafting_extras.set_elev(5.0).expect("5 is finite");
    let outcome = follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    let mesh = created_mesh(&app, &outcome);
    let min_z = mesh
        .vertices
        .iter()
        .map(|vertex| vertex.z)
        .fold(f64::INFINITY, f64::min);
    assert!(
        (min_z + 0.5).abs() < 1e-9,
        "the mesh keeps its own z (min {min_z}), not ELEV + 5"
    );
}

#[test]
fn the_profile_and_the_path_stay_exactly_as_they_were() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
    ]);
    follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    for entity in [PROFILE, PATH] {
        assert!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(entity))
                .is_some(),
            "row {entity} remains"
        );
    }
}

#[test]
fn an_empty_selection_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![square(PROFILE)]);
    let before = revision(&app);
    let text = refusal_text(follow_me(&mut app, &mut session, &[]).unwrap_err());
    assert!(text.starts_with("followme.selection_required:"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn two_closed_profiles_are_refused_by_name() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(
            PATH,
            vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)],
            true,
        ),
    ]);
    let before = revision(&app);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH]).unwrap_err());
    assert!(text.starts_with("followme.profile_multiple:"), "{text}");
    assert_eq!(revision(&app), before);
}

#[test]
fn a_selection_without_a_path_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![square(PROFILE)]);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE]).unwrap_err());
    assert!(text.starts_with("followme.path_missing:"), "{text}");
}

#[test]
fn a_closed_path_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(
            PATH,
            vec![
                p3(0.0, 0.0, 0.0),
                p3(2.0, 0.0, 0.0),
                p3(0.0, 2.0, 0.0),
                p3(0.0, 0.0, 0.0),
            ],
            false,
        ),
    ]);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH]).unwrap_err());
    assert!(text.starts_with("followme.path_closed:"), "{text}");
}

#[test]
fn a_reversing_path_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(
            PATH,
            vec![p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)],
            false,
        ),
    ]);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH]).unwrap_err());
    assert!(text.starts_with("followme.path_reversal:"), "{text}");
}

#[test]
fn a_circle_profile_sweeps_as_a_polygonal_ring() {
    let (mut app, mut session) = app_with(vec![
        ent(
            PROFILE,
            SemanticGeometry::Circle {
                center: p3(0.0, 0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 1.0,
            },
            PROFILE as i64,
        ),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 4.0)], false),
    ]);
    let outcome = follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    let mesh = created_mesh(&app, &outcome);
    // A 48-gon of radius 1 has area 24·sin(7.5°) = 3.13263; over 4 that is
    // 12.5305. The ring is polygonal, exactly like a SketchUp circle.
    let expected = 4.0 * 24.0 * (std::f64::consts::TAU / 48.0).sin();
    assert!(
        (volume(&mesh) - expected).abs() < 1e-6,
        "volume {} vs {expected}",
        volume(&mesh)
    );
    assert_closed_and_oriented(&mesh);
    assert_eq!(mesh.triangles.len(), 48 * 2 + 2 * 46);
}

#[test]
fn a_bulged_profile_or_path_is_refused_by_name() {
    let mut bulged = square(PROFILE);
    if let SemanticGeometry::Polyline { bulges, .. } = &mut bulged.geometry {
        bulges[0] = 0.5;
    }
    let (mut app, mut session) = app_with(vec![
        bulged,
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
    ]);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH]).unwrap_err());
    assert!(text.starts_with("followme.profile_unsupported:"), "{text}");
}

#[test]
fn a_non_planar_profile_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![
        polyline(
            PROFILE,
            vec![
                p3(0.0, 0.0, 0.0),
                p3(1.0, 0.0, 0.0),
                p3(1.0, 1.0, 0.5),
                p3(0.0, 1.0, 0.0),
            ],
            true,
        ),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
    ]);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH]).unwrap_err());
    assert!(text.starts_with("followme.profile_not_planar:"), "{text}");
}

#[test]
fn a_self_intersecting_profile_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![
        polyline(
            PROFILE,
            vec![
                p3(0.0, 0.0, 0.0),
                p3(1.0, 1.0, 0.0),
                p3(1.0, 0.0, 0.0),
                p3(0.0, 1.0, 0.0),
            ],
            true,
        ),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)], false),
    ]);
    let text = refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH]).unwrap_err());
    assert!(
        text.starts_with("followme.profile_self_intersecting:"),
        "{text}"
    );
}

#[test]
fn a_closed_profile_with_a_line_path_is_not_ambiguous() {
    // A closed polyline can only be the profile, so the LINE row is the path.
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        line(PATH, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 2.0)),
    ]);
    let outcome = follow_me(&mut app, &mut session, &[PROFILE, PATH]).expect("the sweep runs");
    let mesh = created_mesh(&app, &outcome);
    assert!(
        (volume(&mesh) - 2.0).abs() < 1e-9,
        "volume {}",
        volume(&mesh)
    );
}

#[test]
fn an_open_polyline_plus_line_rows_is_ambiguous() {
    // Both an open polyline and a LINE row could be the path; the command names
    // the ambiguity instead of picking one.
    let (mut app, mut session) = app_with(vec![
        square(PROFILE),
        polyline(PATH, vec![p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 2.0)], false),
        line(RING_A, p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)),
    ]);
    let text =
        refusal_text(follow_me(&mut app, &mut session, &[PROFILE, PATH, RING_A]).unwrap_err());
    assert!(text.starts_with("followme.ambiguous:"), "{text}");
}
