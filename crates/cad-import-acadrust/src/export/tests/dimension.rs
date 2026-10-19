//! DIMENSION round-trip and export-shape contract tests (ADR 0017 F.1).

use super::*;

fn fixture_bytes(path: &str) -> Arc<[u8]> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join(path)).expect("fixture readable");
    Arc::from(bytes.into_boxed_slice())
}

pub(super) fn dimensions(db: &DrawingDatabase) -> Vec<DimensionGeometry> {
    db.entities()
        .filter_map(|e| match &e.geometry {
            SemanticGeometry::Dimension(d) => Some((**d).clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn style_values() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: 0.0,
        text_vertical: 1,
        text_vertical_position: 0.0,
        suppress_ext1: false,
        suppress_ext2: false,
        suppress_dim1: false,
        suppress_dim2: false,
        linear_factor: 1.0,
        decimals: 4,
        rounding: 0.0,
        postfix: String::new(),
        zero_suppression: 0,
        linear_unit_format: 2,
        decimal_separator: 46,
        text_style_name: "Standard".to_string(),
        text_style: StyleId(0),
        text_font: None,
    }
}

fn aligned_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: p(-21.0, 0.0, 0.0),
            second: p(21.0, 0.0, 0.0),
            dimension_line: p(-21.0, -30.0, 0.0),
        },
        text_middle: p(0.0, 0.0, 0.0),
        text_rotation: 0.0,
        normal: p(0.0, 0.0, 1.0),
    }
}

fn angular_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Angular3Pt,
        points: DimensionPoints::Angular3Pt {
            vertex: p(0.0, 0.0, 0.0),
            first: p(10.0, 0.0, 0.0),
            second: p(0.0, 10.0, 0.0),
            arc_point: p(5.0, 5.0, 0.0),
        },
        text_middle: p(0.0, 0.0, 0.0),
        text_rotation: 0.0,
        normal: p(0.0, 0.0, 1.0),
    }
}

/// A rounded, order-stable rendering of a dimension's definition points plus its
/// group-11 middle point and group-53 rotation, for round-trip comparison.
fn rounded_definition(d: &DimensionGeometry) -> String {
    let r = |p: Point3| format!("{:.6},{:.6},{:.6}", p.x, p.y, p.z);
    let definition = d.definition();
    let points = match &definition.points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => format!("Linear {} {} {}", r(*first), r(*second), r(*dimension_line)),
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => format!(
            "Radial {} {} {:.6}",
            r(*center),
            r(*point_on_arc),
            leader_length
        ),
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => format!(
            "Angular2Ln {} {} {} {} {}",
            r(*line1_start),
            r(*line1_end),
            r(*line2_start),
            r(*line2_end),
            r(*arc_point)
        ),
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => format!(
            "Angular3Pt {} {} {} {}",
            r(*vertex),
            r(*first),
            r(*second),
            r(*arc_point)
        ),
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => format!("Ordinate {} {} {}", r(*feature), r(*leader_end), r(*base)),
    };
    format!(
        "{} | 11={} | 53={:.6}",
        points,
        r(definition.text_middle),
        definition.text_rotation
    )
}

/// The `(group 70, group 42)` pairs of every DIMENSION record in draw order.
fn dimension_records(bytes: &[u8]) -> Vec<(i64, f64)> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim() == "0" && lines[i + 1].trim() == "DIMENSION" {
            let mut flags = None;
            let mut measurement = None;
            let mut j = i + 2;
            while j + 1 < lines.len() {
                let code = lines[j].trim();
                if code == "0" {
                    break;
                }
                match code {
                    "70" if flags.is_none() => {
                        flags = Some(lines[j + 1].trim().parse().expect("group 70 integer"))
                    }
                    "42" => {
                        measurement = Some(lines[j + 1].trim().parse().expect("group 42 double"))
                    }
                    _ => {}
                }
                j += 2;
            }
            records.push((
                flags.expect("DIMENSION group 70"),
                measurement.expect("DIMENSION group 42"),
            ));
        }
        i += 1;
    }
    records
}

/// The group-70 type flags of the first DIMENSION record in the ENTITIES section.
fn dimension_group_70(bytes: &[u8]) -> i64 {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim() == "0" && lines[i + 1].trim() == "DIMENSION" {
            let mut j = i + 2;
            while j + 1 < lines.len() {
                let code = lines[j].trim();
                if code == "0" {
                    break;
                }
                if code == "70" {
                    return lines[j + 1].trim().parse().expect("group 70 is an integer");
                }
                j += 2;
            }
        }
        i += 1;
    }
    panic!("no DIMENSION group 70 found in the exported bytes");
}

/// ADR 0017 F.1: the block-less DIMENSION fixture survives an export/re-import
/// as a DIMENSION, with kind, definition points, style name and text override
/// preserved.
#[test]
fn blockless_dimensions_round_trip_as_dimensions() {
    let imported = import_bytes(&fixture_bytes("fixtures/dxf/qcad-flange/flange.dxf"));
    let before = dimensions(&imported);
    assert!(!before.is_empty(), "the flange fixture carries DIMENSIONs");

    let (bytes, report) = export_db(&imported);
    let reimported = import_bytes(&bytes);
    let after = dimensions(&reimported);

    assert_eq!(
        after.len(),
        before.len(),
        "every dimension survives the round trip as a DIMENSION"
    );
    // Compare as multisets of (kind, rounded measurement, style name,
    // definition points + group-11/group-53, text override).
    let key = |d: &DimensionGeometry| {
        (
            format!("{:?}", d.kind()),
            (d.measurement() * 1e6).round() as i64,
            d.style_name().to_string(),
            rounded_definition(d),
            d.text_override().map(str::to_string),
        )
    };
    let mut a: Vec<_> = before.iter().map(key).collect();
    let mut b: Vec<_> = after.iter().map(key).collect();
    a.sort();
    b.sort();
    assert_eq!(
        a, b,
        "kind/measurement/style/definition-points/override preserved"
    );
    assert!(
        report.counts.converted >= before.len(),
        "each dimension is recorded as converted"
    );
}

/// P5e re-verified against acadrust 0.6.5: the committed QCAD fixture's angular
/// dimensions (group 70 = `34`/`162`, group 42 in **radians**) survive
/// import→export→re-import with kind, style and measurement, and the re-exported
/// group 42 is still the radian value.
///
/// This fixture's dimensions are **block members** placed by INSERTs, so the
/// definition points are block-local before and world-space after the export
/// (INSERT expansion bakes the placement, a documented lossy export behaviour);
/// point preservation is asserted by the block-less flange test instead.
#[test]
fn qcad_angular_dimensions_round_trip_with_radian_group_42() {
    let is_angular = |d: &DimensionGeometry| {
        matches!(
            d.kind(),
            DimensionKind::Angular2Ln | DimensionKind::Angular3Pt
        )
    };
    let imported = import_bytes(&fixture_bytes("fixtures/dxf/qcad-examples/entities.dxf"));
    let before: Vec<_> = dimensions(&imported)
        .into_iter()
        .filter(is_angular)
        .collect();
    assert!(
        !before.is_empty(),
        "the QCAD fixture carries angular dimensions"
    );

    let (bytes, _report) = export_db(&imported);
    let reimported = import_bytes(&bytes);
    let after: Vec<_> = dimensions(&reimported)
        .into_iter()
        .filter(is_angular)
        .collect();
    assert_eq!(
        after.len(),
        before.len(),
        "every angular dimension survives the round trip"
    );

    let key = |d: &DimensionGeometry| {
        (
            format!("{:?}", d.kind()),
            (d.measurement() * 1e6).round() as i64,
            d.style_name().to_string(),
        )
    };
    let mut a: Vec<_> = before.iter().map(key).collect();
    let mut b: Vec<_> = after.iter().map(key).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b, "kind/measurement/style preserved");

    // The written group 42 must stay in radians: every angular DIMENSION record
    // carries one of the imported measurements converted with `to_radians`.
    let mut written: Vec<i64> = dimension_records(&bytes)
        .into_iter()
        .filter(|(flags, _)| matches!(flags & 0x0F, 2 | 5))
        .map(|(_, measurement)| (measurement * 1e6).round() as i64)
        .collect();
    let mut expected: Vec<i64> = before
        .iter()
        .map(|d| (d.measurement().to_radians() * 1e6).round() as i64)
        .collect();
    written.sort();
    expected.sort();
    assert_eq!(
        written, expected,
        "angular group 42 is written in radians for every subtype"
    );
}

/// ADR 0017 F.1: the ExportReport carries the declared shape (content block,
/// DIMSTYLE record, group 42, and the 51/52/71/72 + bit-32 dispositions).
#[test]
fn dimension_export_report_declares_its_shape() {
    let definition = aligned_definition();
    let dimension =
        DimensionGeometry::synthesized(definition, "Standard".to_string(), style_values(), None)
            .expect("synthesized");
    let (_bytes, report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));

    // Each conversion reason is its own entry under the same code.
    let text = report
        .entries
        .iter()
        .filter(|e| e.code == "export.converted.dimension")
        .map(|e| e.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.is_empty(), "a dimension conversion entry exists");
    for needle in [
        "content block",
        "group 42",
        "DIMSTYLE record",
        "group 51/52",
        "group 71/72",
        "group 70 bit 32",
        "bit 128",
        "ACAD_DIMASSOC",
        "NOT-IMPLEMENTED",
    ] {
        assert!(
            text.contains(needle),
            "report message missing '{needle}': {text}"
        );
    }
}

/// ADR 0017 A.3 / review N1: the export forces the world-Z normal, so a `-Z`
/// extrusion dimension's group 210 is dropped and declared in the report.
#[test]
fn negative_z_normal_reports_the_group_210_drop() {
    let mut definition = aligned_definition();
    definition.normal = p(0.0, 0.0, -1.0);
    let dimension =
        DimensionGeometry::synthesized(definition, "Standard".to_string(), style_values(), None)
            .expect("synthesized");
    let (_bytes, report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));
    let text = report
        .entries
        .iter()
        .filter(|e| e.code == "export.converted.dimension")
        .map(|e| e.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("group 210"),
        "the report must declare the group 210 drop: {text}"
    );
}

/// ADR 0017 F.1: the generated content block's members are the `display`
/// children, and the DIMENSION references it (group 2) with group 42 written.
#[test]
fn dimension_content_block_matches_display() {
    let definition = aligned_definition();
    let dimension = DimensionGeometry::synthesized(
        definition.clone(),
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    let display = dimension.display().to_vec();
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));

    // Read back with our own reader: a DIMENSION with a block name, a written
    // measurement, and a block whose members are the display children.
    let entities = read_entities(&bytes);
    let dxf_dimension = entities
        .iter()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d),
            _ => None,
        })
        .expect("a DIMENSION was written");
    let base = dxf_dimension.base();
    assert!(!base.block_name.is_empty(), "group 2 block name written");
    assert!(
        (base.actual_measurement - 42.0).abs() < 1e-9,
        "group 42 measurement written: {}",
        base.actual_measurement
    );
    assert_eq!(base.style_name, "Standard", "group 3 style name written");

    // The content block exists and holds the display members, member for
    // member (not just the same count).
    let members = read_block_members(&bytes, &base.block_name);
    let expected: Vec<String> = display.iter().map(display_descriptor).collect();
    let actual: Vec<String> = members.iter().map(member_descriptor).collect();
    assert_eq!(
        actual, expected,
        "content block members match the display member for member"
    );
}

/// A comparable descriptor for one exported block member.
fn member_descriptor(member: &EntityType) -> String {
    match member {
        EntityType::Line(l) => format!("Line {}->{}", vstr(l.start), vstr(l.end)),
        EntityType::Text(t) => format!("Text {}", t.value),
        EntityType::MText(m) => format!("Text {}", m.value),
        EntityType::PolyfaceMesh(_) => "Mesh".to_string(),
        _ => "other".to_string(),
    }
}

/// The same descriptor space for one display child.
fn display_descriptor(child: &SemanticGeometry) -> String {
    match child {
        SemanticGeometry::Line { start, end } => {
            format!(
                "Line {}->{}",
                vstr(Vector3::new(start.x, start.y, start.z)),
                vstr(Vector3::new(end.x, end.y, end.z))
            )
        }
        SemanticGeometry::Text { text, .. } => format!("Text {text}"),
        SemanticGeometry::Mesh(_) => "Mesh".to_string(),
        _ => "other".to_string(),
    }
}

fn vstr(v: Vector3) -> String {
    format!("{:.4},{:.4},{:.4}", v.x, v.y, v.z)
}

/// ADR 0017 F.1 (second leg): a picture-block dimension's re-imported `display`
/// equals the pre-export `display` member for member.
#[test]
fn picture_branch_display_round_trips_member_for_member() {
    let members = vec![
        SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(10.0, 0.0, 0.0),
        },
        SemanticGeometry::Text {
            text: "42".to_string(),
            position: p(5.0, 1.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            // A real picture-block Text carries the resolved SHX font; the
            // importer resolves the exported STYLE table back to the same key.
            font: Some("txt".to_string()),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
        },
    ];
    let dimension = DimensionGeometry::from_picture(
        aligned_definition(),
        "Standard".to_string(),
        style_values(),
        None,
        members.clone(),
    )
    .expect("from_picture");
    let db = db_with(vec![SemanticGeometry::Dimension(Box::new(dimension))]);
    let (bytes, _report) = export_db(&db);
    let reimported = import_bytes(&bytes);
    let after = dimensions(&reimported);
    assert_eq!(
        after.len(),
        1,
        "the picture dimension re-imports as a DIMENSION"
    );
    assert_eq!(
        after[0].display(),
        members.as_slice(),
        "re-imported display equals the pre-export display member for member"
    );
}

/// ADR 0017 F.1: the written group codes match the ADR's table (3, 42, 11, 1,
/// 13, 14, 10) for an aligned dimension.
#[test]
fn dimension_group_codes_round_trip() {
    let mut definition = aligned_definition();
    definition.text_middle = p(1.0, -2.0, 0.0);
    let dimension = DimensionGeometry::synthesized(
        definition,
        "Standard".to_string(),
        style_values(),
        Some("<> mm".to_string()),
    )
    .expect("synthesized");
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));
    let entities = read_entities(&bytes);
    let dimension = entities
        .iter()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d),
            _ => None,
        })
        .expect("a DIMENSION was written");
    let base = dimension.base();
    assert_eq!(base.style_name, "Standard"); // group 3
    assert!((base.actual_measurement - 42.0).abs() < 1e-9); // group 42
    assert!((base.text_middle_point.x - 1.0).abs() < 1e-9); // group 11
    assert!((base.text_middle_point.y + 2.0).abs() < 1e-9); // group 11
    assert_eq!(base.text_override(), Some("<> mm")); // group 1
    match dimension.as_ref() {
        acadrust::entities::Dimension::Aligned(aligned) => {
            assert!((aligned.first_point.x + 21.0).abs() < 1e-9); // group 13
            assert!((aligned.second_point.x - 21.0).abs() < 1e-9); // group 14
            assert!((aligned.definition_point.y + 30.0).abs() < 1e-9); // group 10
        }
        other => panic!("expected an aligned dimension, got {other:?}"),
    }
}

/// ADR 0017 F.4 / review N3: the import→synthesis hand-off is pinned by checking
/// that an imported dimension's stored `display` is the synthesis of its own
/// stored `definition` + `style`, and that its drawn text is the measurement
/// formatted through the imported style.
///
/// This does **not** by itself prove the numbers match the pre-P5a import: both
/// paths call the one shared builder, so comparing them would be a tautology.
/// The unchanged-numbers evidence is the retained numeric synthesis tests.
#[test]
fn flange_dimension_display_is_the_synthesis_of_its_own_definition() {
    let imported = import_bytes(&fixture_bytes("fixtures/dxf/qcad-flange/flange.dxf"));
    let before = dimensions(&imported);
    assert!(!before.is_empty(), "the flange fixture carries DIMENSIONs");
    for dimension in &before {
        let rebuilt = DimensionGeometry::synthesized(
            dimension.definition().clone(),
            dimension.style_name().to_string(),
            dimension.style().clone(),
            dimension.text_override().map(str::to_string),
        )
        .expect("re-synthesized");
        assert_eq!(
            dimension.display(),
            rebuilt.display(),
            "display is the shared synthesis of the imported definition and style"
        );
        assert_eq!(dimension.measurement(), rebuilt.measurement());

        // Non-trivial hand-off check: the drawn text is the imported measurement
        // formatted through the imported style.
        let (prefix, suffix) = match dimension.kind() {
            DimensionKind::Radius => ("R", ""),
            DimensionKind::Diameter => ("\u{2300}", ""),
            DimensionKind::Angular2Ln | DimensionKind::Angular3Pt => ("", "\u{00B0}"),
            _ => ("", ""),
        };
        let expected = resolve_measurement_text(
            dimension.text_override(),
            prefix,
            suffix,
            dimension.measurement(),
            dimension.style(),
        );
        let drawn = dimension.display().iter().find_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(
            drawn.as_deref(),
            Some(expected.as_str()),
            "the drawn text must be the imported measurement through the imported style"
        );
    }
}

/// ADR 0017 A.5 / F.1: group 70 carries the subtype flag with bit 32 absent and
/// the 0x80 (text user-positioned) bit clear — our creations never user-position
/// text.
#[test]
fn dimension_group_70_flags() {
    // The acadrust writer's per-kind constants: Aligned = 1, Linear = 0.
    let aligned = DimensionGeometry::synthesized(
        aligned_definition(),
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(aligned)));
    let flags = dimension_group_70(&bytes);
    assert_eq!(flags & 0x0F, 1, "aligned subtype flag: {flags:#x}");
    assert_eq!(flags & 0x20, 0, "bit 32 must be absent: {flags:#x}");
    assert_eq!(flags & 0x80, 0, "0x80 must be clear: {flags:#x}");

    let mut linear_definition = aligned_definition();
    linear_definition.kind = DimensionKind::Linear { rotation: 0.0 };
    let linear = DimensionGeometry::synthesized(
        linear_definition,
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(linear)));
    let flags = dimension_group_70(&bytes);
    assert_eq!(flags & 0x0F, 0, "linear subtype flag: {flags:#x}");
    assert_eq!(flags & 0x20, 0, "bit 32 must be absent: {flags:#x}");
    assert_eq!(flags & 0x80, 0, "0x80 must be clear: {flags:#x}");

    // Angular subtypes are the ones AutoCAD writes as 34/37 (2|32, 5|32) when
    // the dimension owns its `*D` block. The acadrust 0.6.5 writer still has no
    // way to set bit 32, so the recorded deviation (ADR 0017 A.5 / decision D.4)
    // holds: the emitted flag is exactly 2 or 5.
    let angular_2ln = DimensionGeometry::synthesized(
        DimensionDefinition {
            kind: DimensionKind::Angular2Ln,
            points: DimensionPoints::Angular2Ln {
                line1_start: p(0.0, 0.0, 0.0),
                line1_end: p(10.0, 0.0, 0.0),
                line2_start: p(10.0, 0.0, 0.0),
                line2_end: p(10.0, 10.0, 0.0),
                arc_point: p(7.0, 2.0, 0.0),
            },
            text_middle: p(0.0, 0.0, 0.0),
            text_rotation: 0.0,
            normal: p(0.0, 0.0, 1.0),
        },
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(angular_2ln)));
    let flags = dimension_group_70(&bytes);
    assert_eq!(flags, 2, "angular two-line subtype flag: {flags:#x}");

    let angular_3pt = DimensionGeometry::synthesized(
        angular_definition(),
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(angular_3pt)));
    let flags = dimension_group_70(&bytes);
    assert_eq!(flags, 5, "angular three-point subtype flag: {flags:#x}");
}

/// P5e: group 42 is the DXF measurement unit. For an angular dimension that is
/// **radians** (the committed fixtures carry 0.785398 for 45 degrees), while the
/// drawn text stays in degrees. A degrees value written to group 42 would be a
/// silent unit corruption that no internal round trip would catch.
#[test]
fn angular_export_writes_group_42_in_radians_and_the_text_in_degrees() {
    let dimension = DimensionGeometry::synthesized(
        angular_definition(),
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    assert!(
        (dimension.measurement() - 90.0).abs() < 1e-9,
        "the drawn measurement is degrees: {}",
        dimension.measurement()
    );
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));
    let entities = read_entities(&bytes);
    let dxf_dimension = entities
        .iter()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d),
            _ => None,
        })
        .expect("a DIMENSION was written");
    assert!(
        (dxf_dimension.base().actual_measurement - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "group 42 is 90 degrees in radians: {}",
        dxf_dimension.base().actual_measurement
    );
    let drawn = read_block_members(&bytes, &dxf_dimension.base().block_name)
        .iter()
        .find_map(|member| match member {
            EntityType::MText(text) => Some(text.value.clone()),
            EntityType::Text(text) => Some(text.value.clone()),
            _ => None,
        })
        .expect("the content block carries the measurement text");
    assert!(
        drawn.contains("90"),
        "the drawn text stays in degrees: {drawn}"
    );
}

/// ADR 0017 F.1 / review B2: an Ordinate dimension's written group 42 equals the
/// drawn text, with a datum that is not at the origin (the pre-fix synthesis
/// drew `|feature.x|` while the writer recomputes `|feature - datum|`).
#[test]
fn ordinate_export_group_42_matches_the_drawn_text() {
    let definition = DimensionDefinition {
        kind: DimensionKind::Ordinate { x_ordinate: true },
        points: DimensionPoints::Ordinate {
            feature: p(15.0, 8.0, 0.0),
            leader_end: p(15.0, 12.0, 0.0),
            base: p(5.0, 5.0, 0.0),
        },
        text_middle: p(0.0, 0.0, 0.0),
        text_rotation: 0.0,
        normal: p(0.0, 0.0, 1.0),
    };
    let dimension =
        DimensionGeometry::synthesized(definition, "Standard".to_string(), style_values(), None)
            .expect("synthesized");
    assert!(
        (dimension.measurement() - 10.0).abs() < 1e-9,
        "datum-relative measurement = {}",
        dimension.measurement()
    );
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(dimension.clone())));
    let entities = read_entities(&bytes);
    let dxf_dimension = entities
        .iter()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d),
            _ => None,
        })
        .expect("a DIMENSION was written");
    let base = dxf_dimension.base();
    assert!(
        (base.actual_measurement - 10.0).abs() < 1e-9,
        "written group 42 = {}",
        base.actual_measurement
    );
    let drawn = dimension
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("text child");
    assert_eq!(drawn, "10.0000", "the drawn text must equal group 42");
    // The x-ordinate flag (0x40) is written.
    let flags = dimension_group_70(&bytes);
    assert_eq!(flags & 0x40, 0x40, "x-ordinate flag: {flags:#x}");
}

/// Read the members of a named block from exported bytes.
fn read_block_members(bytes: &[u8], block_name: &str) -> Vec<EntityType> {
    let document = DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .unwrap()
        .read()
        .unwrap();
    document
        .entities_in_block(block_name)
        .cloned()
        .collect::<Vec<_>>()
}

/// P5b (ADR 0017 B.2(b)): the exported DIMSTYLE record carries the snapshot-only
/// values too, not just the display-driving subset, so a re-reader resolves
/// group 3 without falling back to its own `Standard`.
#[test]
fn dimension_dimstyle_record_carries_the_snapshot_only_values() {
    let definition = aligned_definition();
    let dimension =
        DimensionGeometry::synthesized(definition, "Standard".to_string(), style_values(), None)
            .expect("synthesized");
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));
    let document = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    let style = document
        .dim_styles
        .get("Standard")
        .expect("the DIMSTYLE record is written");
    // B.2(a): the resolved snapshot.
    assert!((style.dimtxt - 2.5).abs() < 1e-9);
    assert!((style.dimasz - 2.5).abs() < 1e-9);
    // B.2(b): carried for export fidelity (from the pinned metric Standard).
    assert_eq!(style.dimlunit, 2);
    assert_eq!(style.dimdsep, 46);
    assert!(style.dimtih, "the corrected DIMTIH row");
    assert!(style.dimtoh, "the corrected DIMTOH row");
    assert!(style.dimtofl, "DIMTOFL is a single pinned value");
    assert!((style.dimdli - 3.75).abs() < 1e-9);
}

/// Review N1: `DIMLUNIT`/`DIMDSEP` drive the drawn text (they live in the
/// entity snapshot), so the exported DIMSTYLE record must carry the snapshot's
/// values, not the DB table's. Here the snapshot uses `,` (44) and architectural
/// (4) against a table `Standard` of `.` (46) and decimal (2).
#[test]
fn dimension_dimstyle_record_writes_the_snapshot_dimlunit_and_dimdsep() {
    let mut style = style_values();
    style.decimals = 2;
    style.decimal_separator = 44; // ','
    style.linear_unit_format = 4; // architectural (declared degradation)
    let dimension =
        DimensionGeometry::synthesized(aligned_definition(), "Standard".to_string(), style, None)
            .expect("synthesized");

    // The drawn text used the snapshot separator, so the record must agree.
    let drawn = dimension
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("text child");
    assert_eq!(drawn, "42,00", "the drawn text uses the snapshot DIMDSEP");

    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));
    let document = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    let record = document
        .dim_styles
        .get("Standard")
        .expect("the DIMSTYLE record is written");
    assert_eq!(
        record.dimdsep, 44,
        "DIMDSEP must be the snapshot's value, not the table's 46"
    );
    assert_eq!(
        record.dimlunit, 4,
        "DIMLUNIT must be the snapshot's value, not the table's 2"
    );
}

/// Review N2: a uniform scale scales the style snapshot's sizes, so the exported
/// DIMSTYLE record's `DIMTXT`/`DIMASZ` match the scaled display rather than the
/// pre-scale sizes (a re-import would otherwise render `1/k` text and arrows).
#[test]
fn uniform_scale_scales_the_exported_dimstyle_sizes() {
    let dimension = DimensionGeometry::synthesized(
        aligned_definition(),
        "Standard".to_string(),
        style_values(),
        None,
    )
    .expect("synthesized");
    let scaled = dimension
        .transformed(&Transform3::scale(3.0))
        .expect("scaled");

    // The scaled display's text height is 3 * 2.5 = 7.5, and so is the snapshot.
    let height = scaled
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { height, .. } => Some(*height),
            _ => None,
        })
        .expect("text child");
    assert!((height - 7.5).abs() < 1e-9, "scaled text height = {height}");
    assert!((scaled.style().text_height - 7.5).abs() < 1e-9);
    assert!((scaled.style().arrow_size - 7.5).abs() < 1e-9);

    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(scaled)));
    let document = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    let record = document
        .dim_styles
        .get("Standard")
        .expect("the DIMSTYLE record is written");
    assert!(
        (record.dimtxt - 7.5).abs() < 1e-9,
        "DIMTXT must match the scaled display: {}",
        record.dimtxt
    );
    assert!(
        (record.dimasz - 7.5).abs() < 1e-9,
        "DIMASZ must match the scaled display: {}",
        record.dimasz
    );
}

/// Review N4: the export report is not silent about the pinned-Standard fallback
/// (a style name absent from the table), the `DIMLUNIT` degradation, or the
/// declared NOT-IMPLEMENTED variables written as `DimStyle::new` neutrals.
#[test]
fn dimension_export_report_declares_the_style_fallbacks() {
    let mut style = style_values();
    style.linear_unit_format = 4; // architectural: declared degradation
    let dimension =
        DimensionGeometry::synthesized(aligned_definition(), "Unlisted".to_string(), style, None)
            .expect("synthesized");
    let (_bytes, report) = export_one(SemanticGeometry::Dimension(Box::new(dimension)));
    let text = report
        .entries
        .iter()
        .filter(|e| e.code == "export.converted.dimension")
        .map(|e| e.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("not in the table"),
        "the pinned-Standard fallback must be declared: {text}"
    );
    assert!(
        text.contains("DIMLUNIT"),
        "the DIMLUNIT degradation must be declared: {text}"
    );
    assert!(
        text.contains("NOT-IMPLEMENTED"),
        "the NOT-IMPLEMENTED variables must be declared: {text}"
    );
}
