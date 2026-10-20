//! HATCH creation contract tests (ADR 0018 decision D, P6b).
//!
//! Pins the pre-selection boundary shape (one loop per selected entity), the
//! numeric solid/pattern results, the style dispatch by **filled area**, the named
//! refusal surface (no transaction on any refusal), the one-transaction/one-undo
//! shape, and the first **export round trip of a created hatch**.

use super::*;

use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

// ---------------------------------------------------------------------------
// Fixtures and helpers
// ---------------------------------------------------------------------------

/// A drawing with layer 0 and the given model-space entities.
fn hatch_app(entities: Vec<(u128, SemanticGeometry)>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (index, (id, geometry)) in entities.into_iter().enumerate() {
        b.insert_entity(ent(id, geometry, index as i64)).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([4u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn circle(id: u128, x: f64, y: f64, z: f64, radius: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Circle {
            center: p3(x, y, z),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
    )
}

fn square(id: u128, min: f64, max: f64, z: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Polyline {
            points: vec![
                p3(min, min, z),
                p3(max, min, z),
                p3(max, max, z),
                p3(min, max, z),
            ],
            bulges: vec![0.0; 4],
            closed: true,
        },
    )
}

fn hatch_payload(ids: &[u128]) -> CommandPayload {
    CommandPayload::Hatch {
        refs: ids.iter().copied().map(ref_of).collect(),
    }
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    ids: &[u128],
) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::Hatch, hatch_payload(ids))
}

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn set_flag(session: &mut SessionState, parameter: ToolParameter, value: bool) {
    session
        .set_tool_parameter(parameter, ToolValue::Flag(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

/// Configure a user-defined pattern (the default is a solid fill).
fn user_defined(session: &mut SessionState, spacing: f64, angle_degrees: f64, double: bool) {
    set_number(
        session,
        ToolParameter::HatchPatternType,
        HATCH_PATTERN_TYPE_USER_DEFINED,
    );
    set_number(session, ToolParameter::HatchSpacing, spacing);
    set_number(session, ToolParameter::HatchAngle, angle_degrees);
    set_flag(session, ToolParameter::HatchDouble, double);
}

fn set_style(session: &mut SessionState, style: f64) {
    set_number(session, ToolParameter::HatchStyle, style);
}

fn the_hatch(app: &Application) -> &HatchGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some(hatch.as_ref()),
            _ => None,
        })
        .expect("the drawing holds a hatch")
}

/// The filled area of the derived display: the summed triangle area of the fill
/// mesh, or `0` when the display has no mesh (a pattern fill).
fn filled_area(hatch: &HatchGeometry) -> f64 {
    let mut area = 0.0;
    for geometry in hatch.display() {
        if let SemanticGeometry::Mesh(mesh) = geometry {
            for triangle in &mesh.triangles {
                let a = mesh.vertices[triangle[0] as usize];
                let b = mesh.vertices[triangle[1] as usize];
                let c = mesh.vertices[triangle[2] as usize];
                let ab = Point3 {
                    x: b.x - a.x,
                    y: b.y - a.y,
                    z: b.z - a.z,
                };
                let ac = Point3 {
                    x: c.x - a.x,
                    y: c.y - a.y,
                    z: c.z - a.z,
                };
                let cross = Point3 {
                    x: ab.y * ac.z - ab.z * ac.y,
                    y: ab.z * ac.x - ab.x * ac.z,
                    z: ab.x * ac.y - ab.y * ac.x,
                };
                area += 0.5 * cross.x.hypot(cross.y).hypot(cross.z);
            }
        }
    }
    area
}

/// The pattern polylines of the derived display (open polylines; the boundary
/// loops are closed) and their total length.
fn pattern_lines(hatch: &HatchGeometry) -> (usize, f64) {
    let mut count = 0;
    let mut total = 0.0;
    for geometry in hatch.display() {
        if let SemanticGeometry::Polyline {
            points,
            closed: false,
            ..
        } = geometry
        {
            count += 1;
            for pair in points.windows(2) {
                total += (pair[1].x - pair[0].x)
                    .hypot(pair[1].y - pair[0].y)
                    .hypot(pair[1].z - pair[0].z);
            }
        }
    }
    (count, total)
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .len()
}

/// Run HATCH and assert an explicit refusal that names `needle`, changed nothing
/// and opened no transaction.
fn assert_refused(app: &mut Application, session: &mut SessionState, ids: &[u128], needle: &str) {
    let before = revision(app);
    let depth = undo_depth(app);
    let error = run(app, session, ids).expect_err("the command must refuse");
    let text = format!("{error:?}");
    assert!(text.contains(needle), "expected {needle:?} in {text}");
    assert_eq!(revision(app), before, "a refusal must not write");
    assert_eq!(
        undo_depth(app),
        depth,
        "a refusal must not open a transaction"
    );
}

// ---------------------------------------------------------------------------
// Numeric geometry
// ---------------------------------------------------------------------------

#[test]
fn a_selected_circle_becomes_one_exact_arc_loop_and_a_solid_fill() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    let out = run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    assert_eq!(out.objects.len(), 1, "one hatch entity");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(out.objects[0].0))
        .unwrap();
    assert_eq!(entity.object.type_key, "AcDbHatch");
    assert_eq!(entity.layer, LayerId(0));

    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1, "one loop per selected entity");
    assert_eq!(hatch.style(), HatchStyle::Normal);
    assert_eq!(hatch.pattern_type(), HatchPatternType::Predefined);
    assert!(matches!(hatch.fill(), HatchFill::Solid));

    // The circle loop is exact: four vertices on the circle, each segment a
    // quarter arc (`bulge = tan(pi/8)`).
    let loop_ = &hatch.loops()[0];
    assert_eq!(loop_.points.len(), 4);
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    assert_eq!(loop_.bulges, vec![bulge; 4]);
    for point in &loop_.points {
        assert!(
            (point[0].hypot(point[1]) - 5.0).abs() < 1e-12,
            "every arc vertex lies on the circle: {point:?}"
        );
    }

    // The derived fill area is pi*r^2 within the arc sampling tolerance.
    let area = filled_area(hatch);
    let expected = std::f64::consts::PI * 25.0;
    assert!(
        (area - expected).abs() < 0.5,
        "solid circle area {area} vs {expected}"
    );

    // The **displayed** boundary is the same circle. The arc construction used to
    // place each quarter arc's centre on the apex side, mirroring the arc across
    // its chord: the loop still sampled to its vertices, but the drawn polyline
    // overshot to 2.195*r with straight jumps at every vertex.
    let mut boundary_samples = 0;
    for geometry in hatch.display() {
        let SemanticGeometry::Polyline { closed, .. } = geometry else {
            continue;
        };
        assert!(*closed);
        for point in
            cad_geometry::tessellate_geometry(geometry, cad_geometry::TessellationParams::default())
        {
            let radial = point.x.hypot(point.y) - 5.0;
            assert!(
                radial.abs() < 1e-9,
                "the displayed boundary leaves the circle by {radial}: {point:?}"
            );
            boundary_samples += 1;
        }
    }
    assert!(boundary_samples > 8, "the boundary was not tessellated");

    // The selected boundary entity is retained (no associativity, no deletion).
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn nested_boundaries_dispatch_the_style_by_filled_area() {
    // Two selected entities, one nested: outer r=10, inner r=6. With two loops
    // `Outer` and `Normal` are equivalent (the ADR's Outer defect only shows at
    // three levels); `Ignore` fills through the hole, so its area is the disk.
    let annulus = std::f64::consts::PI * (100.0 - 36.0);
    let disk = std::f64::consts::PI * 100.0;

    for (style, expected) in [
        (HATCH_STYLE_NORMAL, annulus),
        (HATCH_STYLE_OUTER, annulus),
        (HATCH_STYLE_IGNORE, disk),
    ] {
        let (mut app, mut session) = hatch_app(vec![
            circle(1, 0.0, 0.0, 0.0, 10.0),
            circle(2, 0.0, 0.0, 0.0, 6.0),
        ]);
        set_style(&mut session, style);
        run(&mut app, &mut session, &[1, 2]).expect("HATCH succeeds");
        let hatch = the_hatch(&app);
        assert_eq!(hatch.loops().len(), 2, "each entity is one loop");
        let area = filled_area(hatch);
        assert!(
            (area - expected).abs() < 0.75,
            "style {style} area {area} vs {expected}"
        );
    }
}

#[test]
fn a_three_level_nesting_separates_outer_from_normal() {
    // The ADR's `Outer` defect only appears at three levels: Normal keeps depths 1
    // and 3 (100pi - 36pi + 9pi), Outer keeps depth 1 only (64pi), Ignore keeps
    // the outer loop only (100pi).
    let normal = std::f64::consts::PI * (100.0 - 36.0 + 9.0);
    let outer = std::f64::consts::PI * (100.0 - 36.0);
    let ignore = std::f64::consts::PI * 100.0;
    for (style, expected) in [
        (HATCH_STYLE_NORMAL, normal),
        (HATCH_STYLE_OUTER, outer),
        (HATCH_STYLE_IGNORE, ignore),
    ] {
        let (mut app, mut session) = hatch_app(vec![
            circle(1, 0.0, 0.0, 0.0, 10.0),
            circle(2, 0.0, 0.0, 0.0, 6.0),
            circle(3, 0.0, 0.0, 0.0, 3.0),
        ]);
        set_style(&mut session, style);
        run(&mut app, &mut session, &[1, 2, 3]).expect("HATCH succeeds");
        let hatch = the_hatch(&app);
        assert_eq!(hatch.loops().len(), 3);
        let area = filled_area(hatch);
        assert!(
            (area - expected).abs() < 0.75,
            "style {style} area {area} vs {expected}"
        );
    }
}

#[test]
fn a_user_defined_pattern_synthesizes_the_expected_line_count_and_length() {
    // Square boundary [0.5, 10.5]^2, spacing 2 and angle 0: the pattern lines sit
    // at y = 2, 4, 6, 8, 10, each exactly 10 long.
    let (mut app, mut session) = hatch_app(vec![square(1, 0.5, 10.5, 0.0)]);
    user_defined(&mut session, 2.0, 0.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.pattern_type(), HatchPatternType::UserDefined);
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    assert_eq!(pattern.name, "USER");
    assert!((pattern.scale - 2.0).abs() < 1e-12);
    assert!((pattern.angle - 0.0).abs() < 1e-12);
    assert!(!pattern.double);
    // Exactly the import path's user-defined synthesis: one row at offset (0, 1).
    assert_eq!(pattern.lines.len(), 1);
    assert_eq!(pattern.lines[0].offset_x, 0.0);
    assert_eq!(pattern.lines[0].offset_y, 1.0);
    assert!(pattern.lines[0].dashes.is_empty());

    let (count, total) = pattern_lines(hatch);
    assert_eq!(count, 5, "five horizontal lines");
    assert!((total - 50.0).abs() < 1e-9, "total length {total}");
    // A pattern fill draws no solid mesh.
    assert_eq!(filled_area(hatch), 0.0);

    // The double flag adds the perpendicular family (five more lines).
    let (mut app, mut session) = hatch_app(vec![square(1, 0.5, 10.5, 0.0)]);
    user_defined(&mut session, 2.0, 0.0, true);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let (count, total) = pattern_lines(the_hatch(&app));
    assert_eq!(count, 10, "two perpendicular families");
    assert!((total - 100.0).abs() < 1e-9, "total length {total}");
}

#[test]
fn a_rotated_user_defined_pattern_stays_perpendicular_to_its_angle() {
    // Angle 90 degrees rotates the family to vertical lines at x = 2, 4, 6, 8, 10.
    let (mut app, mut session) = hatch_app(vec![square(1, 0.5, 10.5, 0.0)]);
    user_defined(&mut session, 2.0, 90.0, false);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let (count, total) = pattern_lines(the_hatch(&app));
    assert_eq!(count, 5);
    assert!((total - 50.0).abs() < 1e-9, "total length {total}");
}

#[test]
fn a_full_ellipse_is_one_sampled_loop_and_fills_its_area() {
    // A full ellipse is a legal boundary, sampled as chords (declared: a true
    // ellipse is not a circular arc, so no bulge loop is fabricated).
    let ellipse = SemanticGeometry::Ellipse {
        center: point(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        major_axis: p3(5.0, 0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let (mut app, mut session) = hatch_app(vec![(1, ellipse)]);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    assert_eq!(hatch.loops()[0].points.len(), 64, "chord-sampled boundary");
    assert!(hatch.loops()[0].bulges.iter().all(|bulge| *bulge == 0.0));
    let area = filled_area(hatch);
    let expected = std::f64::consts::PI * 5.0 * 2.5;
    assert!(
        (area - expected).abs() < 0.3,
        "ellipse area {area} vs {expected}"
    );
}

#[test]
fn a_polyline_with_bulges_is_one_loop_with_its_bulges_preserved() {
    // A closed bulged polyline (a half-disc-ish shape) keeps its points/bulges;
    // only a created circle is rewritten into exact arc segments.
    let bulged = SemanticGeometry::Polyline {
        points: vec![p3(-5.0, 0.0, 0.0), p3(5.0, 0.0, 0.0), p3(0.0, 5.0, 0.0)],
        bulges: vec![1.0, 0.0, 0.0],
        closed: true,
    };
    let (mut app, mut session) = hatch_app(vec![(1, bulged)]);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(hatch.loops().len(), 1);
    assert_eq!(hatch.loops()[0].points.len(), 3);
    assert_eq!(hatch.loops()[0].bulges, vec![1.0, 0.0, 0.0]);
    assert!(filled_area(hatch) > 0.0, "the bulged loop is filled");
}

// ---------------------------------------------------------------------------
// Refusals (explicit, named, no transaction)
// ---------------------------------------------------------------------------

#[test]
fn an_empty_selection_is_refused_by_name() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    assert_refused(&mut app, &mut session, &[], "at least one selected");
}

#[test]
fn unsupported_boundary_kinds_are_refused_by_name() {
    // LINE has no boundary stitcher.
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
    )]);
    assert_refused(&mut app, &mut session, &[1], "AcDbLine");

    // An open polyline is not a closed loop.
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(5.0, 0.0), point(5.0, 5.0)],
            bulges: vec![0.0; 3],
            closed: false,
        },
    )]);
    assert_refused(&mut app, &mut session, &[1], "open polyline");

    // An ellipse arc is not a closed loop.
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::PI,
        },
    )]);
    assert_refused(&mut app, &mut session, &[1], "arc, not a closed loop");

    // A spline boundary is NOT IMPLEMENTED.
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                point(0.0, 0.0),
                point(1.0, 2.0),
                point(2.0, -2.0),
                point(3.0, 0.0),
            ],
            weights: vec![1.0; 4],
        },
    )]);
    assert_refused(&mut app, &mut session, &[1], "AcDbSpline");
}

#[test]
fn a_selection_that_is_not_coplanar_with_one_normal_is_refused() {
    // A polyline at z=5 is parallel to the circle's plane but not on it.
    let (mut app, mut session) = hatch_app(vec![
        circle(1, 0.0, 0.0, 0.0, 5.0),
        square(2, -2.0, 2.0, 5.0),
    ]);
    assert_refused(&mut app, &mut session, &[1, 2], "coplanar");

    // Two circles with different declared normals contradict each other.
    let mut x_normal = circle(2, 0.0, 0.0, 0.0, 3.0);
    if let (_, SemanticGeometry::Circle { normal, .. }) = &mut x_normal {
        *normal = Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        };
    }
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0), x_normal]);
    assert_refused(&mut app, &mut session, &[1, 2], "single consistent normal");
}

#[test]
fn the_creation_path_boundary_defects_refuse_the_whole_command() {
    // Self-intersecting boundary (a bowtie quad whose Newell vector is non-zero,
    // so it reaches the creation-path validator rather than the plane check).
    let bowtie = SemanticGeometry::Polyline {
        points: vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(2.0, 8.0),
            point(10.0, 8.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
    };
    let (mut app, mut session) = hatch_app(vec![(1, bowtie)]);
    assert_refused(&mut app, &mut session, &[1], "self-intersecting");

    // Two circles touching at one point (tangent/shared vertex).
    let (mut app, mut session) = hatch_app(vec![
        circle(1, 0.0, 0.0, 0.0, 5.0),
        circle(2, 10.0, 0.0, 0.0, 5.0),
    ]);
    assert_refused(
        &mut app,
        &mut session,
        &[1, 2],
        "touch or partially overlap",
    );

    // Two identical circles are duplicates; the touch check fires first, so the
    // named refusal is the same (the ADR lists duplicate loops as a defect).
    let (mut app, mut session) = hatch_app(vec![
        circle(1, 0.0, 0.0, 0.0, 5.0),
        circle(2, 0.0, 0.0, 0.0, 5.0),
    ]);
    assert_refused(
        &mut app,
        &mut session,
        &[1, 2],
        "touch or partially overlap",
    );

    // A non-finite boundary coordinate is refused before anything is built.
    let (mut app, mut session) = hatch_app(vec![circle(1, f64::NAN, 0.0, 0.0, 5.0)]);
    assert_refused(&mut app, &mut session, &[1], "non-finite");

    // A collinear closed polyline has no plane and no area.
    let (mut app, mut session) = hatch_app(vec![(
        1,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(1.0, 0.0), point(2.0, 0.0)],
            bulges: vec![0.0; 3],
            closed: true,
        },
    )]);
    assert_refused(&mut app, &mut session, &[1], "degenerate");
}

#[test]
fn a_block_instance_or_sub_element_boundary_is_refused() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    let mut reference = ref_of(1);
    reference.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge".into(),
        topology_revision: Revision(0),
    });
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Hatch,
        CommandPayload::Hatch {
            refs: vec![reference],
        },
    )
    .expect_err("a sub-element boundary must be refused");
    assert!(
        format!("{error:?}").contains("model-space entity"),
        "{error:?}"
    );
}

#[test]
fn hatch_is_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::Hatch.requires_work_mode());
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = run(&mut app, &mut session, &[1]).expect_err("Viewer must refuse");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}

#[test]
fn a_missing_active_layer_is_refused_without_a_transaction() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    session.active_layer = LayerId(9);
    assert_refused(&mut app, &mut session, &[1], "layer 9 does not exist");
}

#[test]
fn an_out_of_range_parameter_is_refused_by_name_before_any_write() {
    let (app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    // The typed channel refuses the value at set time, naming the parameter.
    let error = session
        .set_tool_parameter(ToolParameter::HatchSpacing, ToolValue::Number(0.0))
        .expect_err("a zero spacing has no line family");
    let text = format!("{error:?}");
    assert!(text.contains("hatchSpacing"), "{text}");
    // Nothing was written and the stored value is still the valid default.
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(
        session.tool_parameter(ToolParameter::HatchSpacing),
        ToolValue::Number(1.0)
    );
}

// ---------------------------------------------------------------------------
// Transaction shape
// ---------------------------------------------------------------------------

#[test]
fn hatch_inserts_one_entity_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = hatch_app(vec![circle(1, 0.0, 0.0, 0.0, 5.0)]);
    let before = revision(&app);
    let out = run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    assert_eq!(out.objects.len(), 1, "one inserted entity");
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(changes.changes.len(), 1, "one inserted entity");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    // One undo removes the hatch and restores the boundary-only drawing.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "the boundary entity remains");
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// Export round trip (the first time a **created** hatch is written)
// ---------------------------------------------------------------------------

#[test]
fn a_created_user_defined_hatch_exports_and_reimports_as_a_hatch() {
    let (mut app, mut session) = hatch_app(vec![
        circle(1, 0.0, 0.0, 0.0, 10.0),
        circle(2, 0.0, 0.0, 0.0, 6.0),
    ]);
    user_defined(&mut session, 2.5, 30.0, true);
    set_style(&mut session, HATCH_STYLE_OUTER);
    run(&mut app, &mut session, &[1, 2]).expect("HATCH succeeds");
    let created = the_hatch(&app).clone();
    assert_eq!(created.loops().len(), 2);

    let (bytes, report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF");
    assert!(
        text.lines().any(|line| line.trim() == "HATCH"),
        "a real HATCH record is written"
    );
    assert!(
        report.counts.converted >= 1,
        "a hatch is always Converted (declared conversions): {:?}",
        report.counts
    );

    // Re-import the exported bytes and find the hatch.
    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports");
    let model = imported.database.model_space();
    let (entity, hatch) = model
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some((*entity, hatch.as_ref())),
            _ => None,
        })
        .expect("the export re-imports as a first-class hatch (not flattened)");

    assert_eq!(entity.object.type_key, "AcDbHatch");
    assert_eq!(hatch.style(), HatchStyle::Outer);
    assert_eq!(hatch.pattern_type(), HatchPatternType::UserDefined);
    assert_eq!(hatch.loops().len(), 2, "both boundary loops survive");
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    assert_eq!(pattern.name, "USER");
    assert!((pattern.scale - 2.5).abs() < 1e-9);
    assert!((pattern.angle - 30.0f64.to_radians()).abs() < 1e-9);
    assert!(pattern.double);
    assert_eq!(pattern.lines.len(), 1, "the synthesized row round-trips");
}

/// A polyline boundary in a plane parallel to world XY is 2D data: its winding
/// must not flip the hatch plane. The plane used to be derived from the Newell
/// normal's sign, so a clockwise-wound loop produced a `-Z` normal and every
/// bulge was then read in a mirrored frame — the created loop (and its fill)
/// silently described the inward scallop instead of the outward arcs.
#[test]
fn a_clockwise_z_flat_polyline_keeps_the_plane_and_the_bulge_side() {
    let radius = 5.0;
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    // Clockwise around +Z, so the outward quarter arcs carry `-tan(pi/8)`.
    let mut points: Vec<Point3> = (0..4)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::FRAC_PI_2;
            let (sin, cos) = angle.sin_cos();
            p3(radius * cos, radius * sin, 5.0)
        })
        .collect();
    points.reverse();
    let boundary = SemanticGeometry::Polyline {
        points,
        bulges: vec![-bulge; 4],
        closed: true,
    };
    let (mut app, mut session) = hatch_app(vec![(1, boundary)]);
    run(&mut app, &mut session, &[1]).expect("HATCH succeeds");
    let hatch = the_hatch(&app);
    assert_eq!(
        hatch.plane().normal.z,
        1.0,
        "the normal is +Z, not the winding"
    );
    assert_eq!(hatch.plane().elevation, 5.0);
    assert_eq!(hatch.loops()[0].bulges, vec![-bulge; 4]);

    // The displayed boundary is the radius-5 circle in the z = 5 plane.
    let mut samples = 0;
    for geometry in hatch.display() {
        if !matches!(geometry, SemanticGeometry::Polyline { .. }) {
            continue;
        }
        for point in
            cad_geometry::tessellate_geometry(geometry, cad_geometry::TessellationParams::default())
        {
            assert!(
                (point.z - 5.0).abs() < 1e-9,
                "boundary left the plane: {point:?}"
            );
            assert!(
                (point.x.hypot(point.y) - radius).abs() < 1e-9,
                "boundary left the circle: {point:?}"
            );
            samples += 1;
        }
    }
    assert!(samples > 8, "the boundary was not tessellated");
}
