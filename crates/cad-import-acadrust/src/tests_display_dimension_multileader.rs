//! Display-closure contract tests: DIMENSION large-radial jog/leader and MULTILEADER detail.

use super::*;

fn request() -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    }
}

fn base(kind: acadrust::entities::DimensionType) -> acadrust::entities::DimensionBase {
    let mut base = acadrust::entities::DimensionBase::new(kind);
    base.normal = acadrust::types::Vector3::UNIT_Z;
    base
}

fn count_lines(geometry: &SemanticGeometry) -> usize {
    match geometry {
        SemanticGeometry::Line { .. } => 1,
        SemanticGeometry::Compound(children) => children.iter().map(count_lines).sum(),
        _ => 0,
    }
}

fn count_meshes(geometry: &SemanticGeometry) -> usize {
    match geometry {
        SemanticGeometry::Mesh(_) => 1,
        SemanticGeometry::Compound(children) => children.iter().map(count_meshes).sum(),
        _ => 0,
    }
}

fn has_polyline(geometry: &SemanticGeometry) -> bool {
    match geometry {
        SemanticGeometry::Polyline { .. } => true,
        SemanticGeometry::Compound(children) => children.iter().any(has_polyline),
        _ => false,
    }
}

fn has_spline(geometry: &SemanticGeometry) -> bool {
    match geometry {
        SemanticGeometry::Spline { .. } => true,
        SemanticGeometry::Compound(children) => children.iter().any(has_spline),
        _ => false,
    }
}

fn find_text(geometry: &SemanticGeometry) -> Option<&SemanticGeometry> {
    match geometry {
        SemanticGeometry::Text { .. } => Some(geometry),
        SemanticGeometry::Compound(children) => children.iter().find_map(find_text),
        _ => None,
    }
}

// ---- DIMENSION ----

#[test]
fn large_radial_jog_uses_the_jog_angle() {
    let req = request();
    let acad = acadrust::CadDocument::new();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut dim = acadrust::entities::DimensionLargeRadial {
        base: base(acadrust::entities::DimensionType::LargeRadial),
        override_center: acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        jog_point: acadrust::types::Vector3::new(5.0, 0.0, 0.0),
        chord_point: acadrust::types::Vector3::new(10.0, 0.0, 0.0),
        jog_angle: 0.0,
        ..Default::default()
    };

    // A jog angle already pointing at the chord collapses to one straight
    // segment: radial line plus the chord.
    let (straight, straight_status) =
        builder.dimension_semantics(&acadrust::entities::Dimension::LargeRadial(dim.clone()));
    // A 90-degree jog bends the segment away from the chord and needs a
    // connector: radial line plus the jog plus the connector.
    dim.jog_angle = std::f64::consts::FRAC_PI_2;
    let (jogged, jogged_status) =
        builder.dimension_semantics(&acadrust::entities::Dimension::LargeRadial(dim));

    assert_ne!(
        straight, jogged,
        "the jog angle must change the synthesized jog geometry"
    );
    assert_eq!(count_lines(&straight), 2);
    assert_eq!(count_lines(&jogged), 3);

    match jogged_status {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r.contains("jog_angle")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    assert!(matches!(straight_status, Completeness::Partial(_)));
}

#[test]
fn arc_length_dimension_draws_leader_when_present() {
    let req = request();
    let acad = acadrust::CadDocument::new();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut dim = acadrust::entities::DimensionArc {
        base: base(acadrust::entities::DimensionType::ArcLength),
        center_point: acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        first_extension_point: acadrust::types::Vector3::new(6.0, 0.0, 0.0),
        second_extension_point: acadrust::types::Vector3::new(0.0, 5.5, 0.0),
        arc_start_parameter: 0.0,
        arc_end_parameter: std::f64::consts::FRAC_PI_2,
        has_leader: false,
        ..Default::default()
    };

    let (without, _) =
        builder.dimension_semantics(&acadrust::entities::Dimension::Arc(dim.clone()));
    let lines_without = count_lines(&without);

    dim.has_leader = true;
    dim.first_leader_point = acadrust::types::Vector3::new(6.0, 1.0, 0.0);
    dim.second_leader_point = acadrust::types::Vector3::new(9.0, 1.0, 0.0);
    let (with, status) = builder.dimension_semantics(&acadrust::entities::Dimension::Arc(dim));

    assert_eq!(
        count_lines(&with),
        lines_without + 1,
        "the arc-length leader adds exactly one line"
    );
    assert_eq!(status, Completeness::Complete);
}

#[test]
fn partial_arc_dimension_is_reported_partial() {
    let req = request();
    let acad = acadrust::CadDocument::new();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let dim = acadrust::entities::DimensionArc {
        base: base(acadrust::entities::DimensionType::ArcLength),
        center_point: acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        first_extension_point: acadrust::types::Vector3::new(6.0, 0.0, 0.0),
        second_extension_point: acadrust::types::Vector3::new(0.0, 5.5, 0.0),
        arc_start_parameter: 0.0,
        arc_end_parameter: std::f64::consts::FRAC_PI_2,
        is_partial: true,
        ..Default::default()
    };

    let (_geometry, status) = builder.dimension_semantics(&acadrust::entities::Dimension::Arc(dim));

    match status {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r.contains("partial arc dimension")),
                "{reasons:?}"
            );
            assert!(
                reasons
                    .iter()
                    .any(|r| r.contains("partial-arc marker/leader")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

// ---- MULTILEADER ----

/// A document carrying one `MultiLeaderStyle` in its object map, plus a
/// MULTILEADER that references it. Entity values are deliberately set to the
/// opposite of the style so style resolution is observable.
fn mleader_with_style() -> (acadrust::CadDocument, acadrust::entities::MultiLeader) {
    let mut doc = acadrust::CadDocument::new();
    let handle = doc.allocate_handle();
    let mut style = acadrust::objects::MultiLeaderStyle::new("Test");
    style.handle = handle;
    style.path_type = acadrust::objects::MultiLeaderPathType::Spline;
    style.arrowhead_size = 0.5;
    style.enable_dogleg = true;
    style.landing_distance = 0.4;
    style.landing_gap = 0.05;
    doc.objects.insert(
        handle,
        acadrust::objects::ObjectType::MultiLeaderStyle(style),
    );

    let mut ml = acadrust::entities::MultiLeader::new();
    ml.style_handle = Some(handle);
    // No override flags: these entity values must lose to the style.
    ml.path_type = acadrust::entities::MultiLeaderPathType::StraightLineSegments;
    ml.arrowhead_size = 0.0;
    ml.enable_dogleg = false;
    let root = ml.add_leader_root();
    root.connection_point = acadrust::types::Vector3::new(9.0, 0.0, 0.0);
    root.landing_distance = 0.4;
    root.create_line(vec![
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(1.0, 1.0, 0.0),
        acadrust::types::Vector3::new(3.0, 1.5, 0.0),
        acadrust::types::Vector3::new(5.0, 1.0, 0.0),
    ]);
    (doc, ml)
}

#[test]
fn multileader_resolves_style_for_spline_path_and_arrowheads() {
    let (doc, ml) = mleader_with_style();
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, completeness) = builder.multileader_semantics(&ml);

    // The style's Spline wins over the entity's straight default.
    assert!(
        has_spline(&geometry),
        "expected a spline path: {geometry:?}"
    );
    assert!(!has_polyline(&geometry));
    // The style arrowhead size (0.5) wins over the entity's 0.0: one arrowhead.
    assert_eq!(count_meshes(&geometry), 1);
    // The style's dogleg default (entity disabled, no override) draws a landing.
    assert!(count_lines(&geometry) >= 1);

    match completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("colour")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn multileader_invisible_path_draws_no_leader_line() {
    let (mut doc, ml) = mleader_with_style();
    if let Some(acadrust::objects::ObjectType::MultiLeaderStyle(style)) =
        doc.objects.get_mut(&ml.style_handle.expect("style handle"))
    {
        style.path_type = acadrust::objects::MultiLeaderPathType::Invisible;
    }
    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);

    assert!(!has_spline(&geometry));
    assert!(!has_polyline(&geometry));
    // An invisible leader line draws no arrowhead either.
    assert_eq!(count_meshes(&geometry), 0);
}

#[test]
fn multileader_text_uses_the_attachment_enums() {
    let (doc, mut ml) = mleader_with_style();
    ml.context.has_text_contents = true;
    ml.context.text_string = "Note".into();
    ml.context.text_location = acadrust::types::Vector3::new(10.0, 2.0, 0.0);
    ml.context.text_height = 0.25;
    ml.text_attachment_point = acadrust::entities::TextAttachmentPointType::Right;
    ml.text_left_attachment = acadrust::entities::TextAttachmentType::BottomOfBottomLine;

    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);

    match find_text(&geometry).expect("text child") {
        SemanticGeometry::Text {
            h_align, v_align, ..
        } => {
            assert_eq!(*h_align, TextAlignH::Right);
            assert_eq!(*v_align, TextAlignV::Bottom);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn multileader_dogleg_follows_the_enable_flag() {
    let (doc, mut ml) = mleader_with_style();
    // Force the entity override on and disable the dogleg.
    ml.property_override_flags =
        acadrust::entities::MultiLeaderPropertyOverrideFlags::ENABLE_DOGLEG;
    ml.enable_dogleg = false;

    let req = request();
    let builder = ImporterBuilder::new(&req, &doc, ReadStats::default(), compute_identity(&[]));
    let (geometry, _) = builder.multileader_semantics(&ml);
    assert_eq!(
        count_lines(&geometry),
        0,
        "an overridden disabled dogleg draws no landing"
    );
}
