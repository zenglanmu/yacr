//! Display-closure contract tests: TABLE borders/background and non-proxy Extended entities.

use super::*;
use acadrust::entities::{
    ArcAlignedTextData, ExtendedEntity, ExtendedEntityData, GeoPositionMarkerData, PointCloudData,
    RemoteTextData, SectionObjectData,
};

fn make_request() -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

/// A 1x1 table at the origin with a 4 x 3 cell, no content.
fn one_cell_table() -> acadrust::entities::Table {
    let mut table = acadrust::entities::Table::new(acadrust::types::Vector3::ZERO, 1, 1);
    table.set_column_width(0, 4.0);
    table.set_row_height(0, 3.0);
    table
}

fn collect_lines(geometry: &SemanticGeometry, out: &mut Vec<(Point3, Point3)>) {
    match geometry {
        SemanticGeometry::Line { start, end } => out.push((*start, *end)),
        SemanticGeometry::Compound(children) => {
            for child in children {
                collect_lines(child, out);
            }
        }
        _ => {}
    }
}

fn lines(geometry: &SemanticGeometry) -> Vec<(Point3, Point3)> {
    let mut out = Vec::new();
    collect_lines(geometry, &mut out);
    out
}

fn has_parallel_at_y(segments: &[(Point3, Point3)], y: f64) -> bool {
    segments
        .iter()
        .any(|(a, b)| (a.y - y).abs() < 1e-9 && (b.y - y).abs() < 1e-9 && (a.x - b.x).abs() > 1e-9)
}

fn has_horizontal_spanning_y(segments: &[(Point3, Point3)], y: f64) -> bool {
    has_parallel_at_y(segments, y)
}

fn partial_reasons(completeness: &Completeness) -> Vec<String> {
    match completeness {
        Completeness::Partial(reasons) => reasons.clone(),
        other => panic!("expected Partial, got {other:?}"),
    }
}

// ---- PART 1: TABLE borders / background ----

#[test]
fn table_cell_style_hides_invisible_edge_and_doubles_bottom() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut table = one_cell_table();
    let mut style = acadrust::entities::CellStyle::new();
    style.top_border.invisible = true;
    style.bottom_border.double_spacing = 0.5;
    table.rows[0].cells[0].style = Some(style);

    let (geometry, completeness) = builder.table_semantics(&table);
    let segments = lines(&geometry);
    // top dropped; right, left and bottom single + bottom double remain.
    assert_eq!(segments.len(), 4, "segments: {segments:?}");
    assert!(
        !has_horizontal_spanning_y(&segments, 3.0),
        "the invisible top edge must not be drawn: {segments:?}"
    );
    assert!(
        has_parallel_at_y(&segments, 0.5),
        "the bottom double line must be offset by its spacing: {segments:?}"
    );
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("table borders drawn from cell styles")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("per-cell border colours/lineweights and cell background fills")),
        "{reasons:?}"
    );
}

#[test]
fn table_styled_reason_enumerates_unhonoured_border_fields() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    // Any styled cell makes the border-fidelity reason the reported one.
    let mut table = one_cell_table();
    table.rows[0].cells[0].style = Some(acadrust::entities::CellStyle::new());

    let (_geometry, completeness) = builder.table_semantics(&table);
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("border_type/override_flags")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("highest-priority style")),
        "{reasons:?}"
    );
}

#[test]
fn table_row_style_is_used_when_the_cell_has_none() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut table = one_cell_table();
    let mut style = acadrust::entities::CellStyle::new();
    style.top_border.invisible = true;
    table.rows[0].style = Some(style);

    let (geometry, _) = builder.table_semantics(&table);
    let segments = lines(&geometry);
    assert!(!has_horizontal_spanning_y(&segments, 3.0), "{segments:?}");
    assert_eq!(segments.len(), 3, "{segments:?}");
}

#[test]
fn table_base_style_is_used_when_the_cell_and_row_are_absent() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut table = one_cell_table();
    let mut style = acadrust::entities::CellStyle::new();
    style.left_border.invisible = true;
    table.base_style = Some(style);

    let (geometry, _) = builder.table_semantics(&table);
    let segments = lines(&geometry);
    assert!(
        !segments
            .iter()
            .any(|(a, b)| { (a.x).abs() < 1e-9 && (b.x).abs() < 1e-9 && (a.y - b.y).abs() > 1e-9 }),
        "the base-style invisible left edge must not be drawn: {segments:?}"
    );
}

#[test]
fn table_style_object_is_resolved_from_objects_by_handle() {
    let mut acad = acadrust::CadDocument::new();
    let handle = acad.allocate_handle();
    let mut style = acadrust::objects::TableStyle::new("S");
    style.handle = handle;
    style.data_row_style.top_border.is_invisible = true;
    acad.objects
        .insert(handle, acadrust::objects::ObjectType::TableStyle(style));

    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut table = one_cell_table();
    table.table_style_handle = Some(handle);

    let (geometry, completeness) = builder.table_semantics(&table);
    let segments = lines(&geometry);
    assert!(
        !has_horizontal_spanning_y(&segments, 3.0),
        "the table-style object's invisible top edge must be honoured: {segments:?}"
    );
    assert_eq!(segments.len(), 3, "{segments:?}");
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("table-style header/title row borders are approximated")),
        "{reasons:?}"
    );
}

#[test]
fn table_merged_interior_edge_is_suppressed() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut table = acadrust::entities::Table::new(acadrust::types::Vector3::ZERO, 1, 2);
    table.set_column_width(0, 2.0);
    table.set_column_width(1, 2.0);
    table.set_row_height(0, 1.0);
    table.rows[0].cells[0].merge_width = 2;

    let (geometry, completeness) = builder.table_semantics(&table);
    let segments = lines(&geometry);
    // The anchor draws only its outer rectangle; the covered cell is skipped,
    // so the interior separator at x == 2 must not appear.
    assert!(
        !segments
            .iter()
            .any(|(a, b)| (a.x - 2.0).abs() < 1e-9 && (b.x - 2.0).abs() < 1e-9),
        "merged interior edge must be suppressed: {segments:?}"
    );
    assert_eq!(segments.len(), 4, "{segments:?}");
    // No style was present, so the plain-grid reason is kept.
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("table grid and cell text drawn")),
        "{reasons:?}"
    );
}

// ---- PART 2: non-proxy Extended entities ----

fn extended(data: ExtendedEntityData) -> ExtendedEntity {
    ExtendedEntity {
        common: EntityCommon::default(),
        data,
    }
}

#[test]
fn extended_remote_text_maps_to_text_complete_when_flags_zero() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::RemoteText(RemoteTextData {
        position: acadrust::types::Vector3::new(1.0, 2.0, 3.0),
        normal: acadrust::types::Vector3::UNIT_Z,
        rotation: 0.5,
        height: 2.5,
        style_handle: acadrust::Handle::NULL,
        style_name: String::new(),
        flags: 0,
        text: "hello".into(),
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    assert_eq!(completeness, Completeness::Complete);
    let SemanticGeometry::Text {
        text,
        position,
        height,
        rotation,
        ..
    } = geometry
    else {
        panic!("expected text geometry");
    };
    assert_eq!(text, "hello");
    assert!((position.x - 1.0).abs() < 1e-9);
    assert!((position.y - 2.0).abs() < 1e-9);
    assert!((height - 2.5).abs() < 1e-9);
    assert!((rotation - 0.5).abs() < 1e-9);
}

#[test]
fn extended_remote_text_reports_unhandled_flags() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::RemoteText(RemoteTextData {
        position: acadrust::types::Vector3::ZERO,
        normal: acadrust::types::Vector3::UNIT_Z,
        rotation: 0.0,
        height: 2.5,
        style_handle: acadrust::Handle::NULL,
        style_name: String::new(),
        flags: 0x6,
        text: "hello".into(),
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    assert!(matches!(geometry, SemanticGeometry::Text { .. }));
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons.iter().any(|r| r.contains("backward")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("upside-down")),
        "{reasons:?}"
    );
}

#[test]
fn extended_remote_text_off_plane_normal_is_partial() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::RemoteText(RemoteTextData {
        position: acadrust::types::Vector3::ZERO,
        normal: acadrust::types::Vector3::new(1.0, 0.0, 0.0),
        rotation: 0.0,
        height: 2.5,
        style_handle: acadrust::Handle::NULL,
        style_name: String::new(),
        flags: 0,
        text: "hello".into(),
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    assert!(matches!(geometry, SemanticGeometry::Text { .. }));
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r == "remote text OCS normal is not applied; text is placed in world XY"),
        "{reasons:?}"
    );
}

#[test]
fn extended_arc_aligned_text_is_partial_with_exact_reason() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::ArcAlignedText(ArcAlignedTextData {
        text: "arc".into(),
        font_name: String::new(),
        big_font_name: String::new(),
        style_name: String::new(),
        center: acadrust::types::Vector3::new(5.0, 6.0, 0.0),
        radius: 10.0,
        x_scale: 1.0,
        text_size: 2.0,
        character_spacing: 0.0,
        offset_from_arc: 0.0,
        right_offset: 0.0,
        left_offset: 0.0,
        start_angle: 0.0,
        end_angle: 1.0,
        reverse: false,
        text_direction: 0,
        alignment: 0,
        text_position: 0,
        bold: false,
        italic: false,
        underlined: false,
        character_set: 0,
        pitch_and_family: 0,
        is_shx: false,
        text_color: 0,
        normal: acadrust::types::Vector3::UNIT_Z,
        wizard_flag: false,
        arc_handle: acadrust::Handle::NULL,
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    let SemanticGeometry::Text { text, position, .. } = geometry else {
        panic!("expected text geometry");
    };
    assert_eq!(text, "arc");
    assert!((position.x - 5.0).abs() < 1e-9);
    assert_eq!(
        completeness,
        Completeness::Partial(vec![
            "arc-aligned placement approximated as linear text".into()
        ])
    );
}

#[test]
fn extended_geo_position_marker_draws_radius_circle_partial() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::GeoPositionMarker(
        GeoPositionMarkerData {
            class_version: 0,
            position: acadrust::types::Vector3::new(7.0, 8.0, 0.0),
            radius: 5.0,
            notes: String::new(),
            landing_gap: 0.0,
            mtext_visible: false,
            text_alignment: 0,
            enable_frame_text: false,
            embedded_mtext: None,
        },
    ));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    let SemanticGeometry::Compound(children) = &geometry else {
        panic!("expected a compound, got {geometry:?}");
    };
    let circle = children.iter().find_map(|child| match child {
        SemanticGeometry::Circle { center, radius, .. } => Some((*center, *radius)),
        _ => None,
    });
    let (center, radius) = circle.expect("a radius circle");
    assert!((center.x - 7.0).abs() < 1e-9 && (center.y - 8.0).abs() < 1e-9);
    assert!((radius - 5.0).abs() < 1e-9);
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("text_alignment") && r.contains("landing_gap")),
        "{reasons:?}"
    );
}

#[test]
fn extended_section_object_draws_polylines_partial() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::SectionObject(SectionObjectData {
        state: 0,
        flags: 0,
        name: "S".into(),
        vertical_direction: acadrust::types::Vector3::UNIT_Z,
        top_height: 0.0,
        bottom_height: 0.0,
        indicator_alpha: 0,
        indicator_color: acadrust::types::Color::ByBlock,
        vertices: vec![
            acadrust::types::Vector3::new(0.0, 0.0, 0.0),
            acadrust::types::Vector3::new(1.0, 0.0, 0.0),
            acadrust::types::Vector3::new(1.0, 1.0, 0.0),
        ],
        back_line_vertices: vec![
            acadrust::types::Vector3::new(0.0, 0.0, 1.0),
            acadrust::types::Vector3::new(1.0, 0.0, 1.0),
            acadrust::types::Vector3::new(1.0, 1.0, 1.0),
        ],
        settings_handle: acadrust::Handle::NULL,
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    let SemanticGeometry::Compound(children) = &geometry else {
        panic!("expected a compound, got {geometry:?}");
    };
    assert_eq!(children.len(), 2, "{children:?}");
    assert!(children
        .iter()
        .all(|child| matches!(child, SemanticGeometry::Polyline { .. })));
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("section object outline drawn")),
        "{reasons:?}"
    );
}

#[test]
fn extended_point_cloud_extents_box_is_partial() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::PointCloud(PointCloudData {
        class_version: 0,
        origin: acadrust::types::Vector3::ZERO,
        saved_filename: String::new(),
        source_files: Vec::new(),
        extents_min: acadrust::types::Vector3::new(-1.0, -2.0, -3.0),
        extents_max: acadrust::types::Vector3::new(1.0, 2.0, 3.0),
        point_count: 0,
        ucs_name: String::new(),
        ucs_origin: acadrust::types::Vector3::ZERO,
        ucs_x_direction: acadrust::types::Vector3::UNIT_X,
        ucs_y_direction: acadrust::types::Vector3::UNIT_Y,
        ucs_z_direction: acadrust::types::Vector3::UNIT_Z,
        definition_handle: acadrust::Handle::NULL,
        reactor_handle: acadrust::Handle::NULL,
        show_intensity: false,
        intensity_scheme: 0,
        minimum_intensity: 0.0,
        maximum_intensity: 0.0,
        low_intensity_threshold: 0.0,
        high_intensity_threshold: 0.0,
        show_clipping: false,
        clippings: Vec::new(),
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    let segments = lines(&geometry);
    assert_eq!(segments.len(), 12, "a box has twelve edges: {segments:?}");
    let reasons = partial_reasons(&completeness);
    assert_eq!(
        reasons,
        vec!["point data is not available from acadrust 0.6.8; extents box shown".to_string()]
    );
}

#[test]
fn extended_point_cloud_degenerate_extents_reports_no_box() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::PointCloud(PointCloudData {
        class_version: 0,
        origin: acadrust::types::Vector3::ZERO,
        saved_filename: String::new(),
        source_files: Vec::new(),
        // A zero-extent box has no drawable edges.
        extents_min: acadrust::types::Vector3::new(1.0, 1.0, 1.0),
        extents_max: acadrust::types::Vector3::new(1.0, 1.0, 1.0),
        point_count: 0,
        ucs_name: String::new(),
        ucs_origin: acadrust::types::Vector3::ZERO,
        ucs_x_direction: acadrust::types::Vector3::UNIT_X,
        ucs_y_direction: acadrust::types::Vector3::UNIT_Y,
        ucs_z_direction: acadrust::types::Vector3::UNIT_Z,
        definition_handle: acadrust::Handle::NULL,
        reactor_handle: acadrust::Handle::NULL,
        show_intensity: false,
        intensity_scheme: 0,
        minimum_intensity: 0.0,
        maximum_intensity: 0.0,
        low_intensity_threshold: 0.0,
        high_intensity_threshold: 0.0,
        show_clipping: false,
        clippings: Vec::new(),
    }));
    let (geometry, completeness) = builder.extended_semantics(&entity);
    assert!(
        matches!(geometry, SemanticGeometry::Opaque { .. }),
        "a degenerate box draws nothing: {geometry:?}"
    );
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons.iter().any(|r| r.contains("degenerate")),
        "{reasons:?}"
    );
    assert!(
        !reasons.iter().any(|r| r.contains("extents box shown")),
        "a degenerate box must not claim an extents box was shown: {reasons:?}"
    );
}

#[test]
fn convert_routes_non_proxy_extended_to_its_semantics() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = EntityType::Extended(Box::new(extended(ExtendedEntityData::ArcAlignedText(
        ArcAlignedTextData {
            text: "arc".into(),
            font_name: String::new(),
            big_font_name: String::new(),
            style_name: String::new(),
            center: acadrust::types::Vector3::ZERO,
            radius: 1.0,
            x_scale: 1.0,
            text_size: 1.0,
            character_spacing: 0.0,
            offset_from_arc: 0.0,
            right_offset: 0.0,
            left_offset: 0.0,
            start_angle: 0.0,
            end_angle: 1.0,
            reverse: false,
            text_direction: 0,
            alignment: 0,
            text_position: 0,
            bold: false,
            italic: false,
            underlined: false,
            character_set: 0,
            pitch_and_family: 0,
            is_shx: false,
            text_color: 0,
            normal: acadrust::types::Vector3::UNIT_Z,
            wizard_flag: false,
            arc_handle: acadrust::Handle::NULL,
        },
    ))));
    let common = entity.common().clone();
    let (geometry, completeness) = builder.convert(&entity, &common);
    assert!(matches!(geometry, SemanticGeometry::Text { .. }));
    assert!(
        matches!(&completeness, Completeness::Partial(reasons)
            if reasons.iter().any(|r| r == "arc-aligned placement approximated as linear text")),
        "{completeness:?}"
    );
}

// ---- ACAD Architecture (AEC): acadrust 0.6.8 `ExtendedEntityData::Arch` ----

fn arch_entity(
    profiles: Vec<Vec<acadrust::types::Vector3>>,
) -> acadrust::classes::arch::ArchEntityData {
    acadrust::classes::arch::ArchEntityData {
        dxf_name: "AEC_WALL".into(),
        cpp_class_name: "AecDbWall".into(),
        kind: acadrust::classes::arch::ArchKind::Wall,
        profiles,
        dwg_type_code: 0,
        dwg_handle_bits: 0,
        dwg_source_version: None,
        raw_dwg_data: None,
        raw_dxf_codes: None,
        fields: Vec::new(),
        preview: Vec::new(),
    }
}

#[test]
fn extended_aec_arch_draws_each_proxy_profile_as_a_polyline_partial() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let square = vec![
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(4.0, 0.0, 0.0),
        acadrust::types::Vector3::new(4.0, 3.0, 0.0),
        acadrust::types::Vector3::new(0.0, 3.0, 0.0),
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
    ];
    let short = vec![acadrust::types::Vector3::new(9.0, 0.0, 0.0)];
    let entity = extended(ExtendedEntityData::Arch(arch_entity(vec![square, short])));
    let (geometry, completeness) = builder.extended_semantics(&entity);

    let SemanticGeometry::Compound(children) = &geometry else {
        panic!("expected a compound, got {geometry:?}");
    };
    assert_eq!(children.len(), 1, "a one-point profile is not drawn");
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = &children[0]
    else {
        panic!("expected a polyline, got {:?}", children[0]);
    };
    assert_eq!(points.len(), 5);
    assert!(bulges.is_empty());
    // The encoded profile carries no closure flag; an open polyline whose last
    // point repeats the first renders identically, so it must not be invented.
    assert!(!closed);
    assert!((points[1].x - 4.0).abs() < 1e-12 && (points[1].y - 0.0).abs() < 1e-12);

    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("proxy-graphics preview profile")),
        "{reasons:?}"
    );
}

#[test]
fn extended_aec_arch_without_profiles_is_opaque_and_named() {
    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let entity = extended(ExtendedEntityData::Arch(arch_entity(Vec::new())));
    let (geometry, completeness) = builder.extended_semantics(&entity);

    let SemanticGeometry::Opaque { type_key, .. } = &geometry else {
        panic!("expected opaque, got {geometry:?}");
    };
    assert_eq!(type_key, "AEC_WALL");
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("no decodable proxy-graphics profile")),
        "{reasons:?}"
    );
}

// ---- AEC straight walls: native start/end/thickness footprint ----

fn wall(
    start: acadrust::Vector3,
    end: acadrust::Vector3,
    width: f64,
) -> acadrust::classes::arch::StraightWallGeometry {
    acadrust::classes::arch::StraightWallGeometry {
        start,
        end,
        width,
        height: 3.0,
    }
}

/// Absolute shoelace area of a closed point ring (first point == last).
fn shoelace_area(points: &[Point3]) -> f64 {
    let mut twice = 0.0;
    for pair in points.windows(2) {
        twice += pair[0].x * pair[1].y - pair[1].x * pair[0].y;
    }
    (twice * 0.5).abs()
}

#[test]
fn aec_straight_wall_footprint_is_an_oriented_rectangle() {
    let geometry = wall(
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(10.0, 0.0, 0.0),
        2.0,
    );
    let outline =
        ImporterBuilder::wall_footprint_for_test(&geometry).expect("a straight wall footprint");
    assert_eq!(outline.len(), 5, "{outline:?}");
    assert_eq!(outline[0], outline[4], "the ring repeats its first point");
    assert!(
        (shoelace_area(&outline) - 20.0).abs() < 1e-9,
        "10 long times 2 thick: {outline:?}"
    );
    let distinct = &outline[..4];
    let cx = distinct.iter().map(|p| p.x).sum::<f64>() / 4.0;
    let cy = distinct.iter().map(|p| p.y).sum::<f64>() / 4.0;
    assert!(
        (cx - 5.0).abs() < 1e-9 && cy.abs() < 1e-9,
        "centroid must be the baseline midpoint: ({cx}, {cy})"
    );
}

#[test]
fn aec_diagonal_wall_footprint_stays_perpendicular() {
    let geometry = wall(
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(10.0, 10.0, 0.0),
        2.0,
    );
    let outline =
        ImporterBuilder::wall_footprint_for_test(&geometry).expect("a straight wall footprint");
    assert_eq!(outline.len(), 5, "{outline:?}");
    // The baseline is (0,0)-(10,10), so it is sqrt(200) long and a width-2
    // rectangle covers sqrt(200) * 2, not 20.
    let expected_area = 200f64.sqrt() * 2.0;
    assert!(
        (shoelace_area(&outline) - expected_area).abs() < 1e-9,
        "{outline:?}"
    );
    // The baseline is y == x, so half the thickness is the perpendicular
    // distance |y - x| / sqrt(2). An axis-aligned rectangle would be off by
    // sqrt(2) here instead.
    for point in &outline {
        let distance = (point.y - point.x).abs() / std::f64::consts::SQRT_2;
        assert!(
            (distance - 1.0).abs() < 1e-9,
            "point {point:?} is {distance} from the baseline, not half the thickness"
        );
    }
}

#[test]
fn aec_wall_footprint_rejects_non_positive_width() {
    let start = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    let end = acadrust::types::Vector3::new(10.0, 0.0, 0.0);
    assert!(ImporterBuilder::wall_footprint_for_test(&wall(start, end, 0.0)).is_none());
    assert!(ImporterBuilder::wall_footprint_for_test(&wall(start, end, -1.0)).is_none());
}
