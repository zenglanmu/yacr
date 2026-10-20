//! N7 adversarial MULTILEADER tests.
//!
//! The self-consistent P7 round-trip fixture cannot observe any declared
//! normalisation: it feeds our own model back into itself. The source fixtures
//! in [`crate::tests_multileader_fixtures`] deliberately disagree with the
//! declared behaviour instead, so these tests pin the normalisations a future
//! change must not silently alter.
//!
//! Split from the fixtures module so both files stay under the repository's
//! 1000-line limit.

use acadrust::entities::{
    LeaderContentType, MultiLeaderPathType as AcadPathType, TextAttachmentDirectionType,
    TextAttachmentPointType, TextAttachmentType,
};
use acadrust::objects::ObjectType as AcadObjectType;
use acadrust::types::{Handle as AcadHandle, Vector3};

use super::*;
use crate::tests_multileader_export::{
    bodies, leader_lines, leader_roots, pairs, records, tail, value, values,
};
use crate::tests_multileader_fixtures::{
    acad_multileaders, adversarial_source_bytes, db_with, empty_text_geometry, empty_text_source,
    export_db, export_one, import_bytes, imported_geometries, read_dxf, rich_geometry, rich_source,
    source_model_with_inconsistent_break_mirrors, with_inconsistent_break_mirrors,
    ADVERSARIAL_MASK_BITS,
};

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn approx_vector(actual: Vector3, expected: Vector3) -> bool {
    approx(actual.x, expected.x) && approx(actual.y, expected.y) && approx(actual.z, expected.z)
}

// ---------------------------------------------------------------------------
// N7: pin the declared normalisations.
// ---------------------------------------------------------------------------

/// Declared: the entity-tail `271` is derived from root 0, never from the source
/// entity-level value. This is a normalisation, **not** a fidelity claim.
#[test]
fn entity_tail_271_is_derived_from_root_zero() {
    let source_bytes = adversarial_source_bytes();
    let source_document = read_dxf(&source_bytes);
    let leaders = acad_multileaders(&source_document);
    let source = rich_source(&leaders);
    // Fixture sanity: the source really does disagree with itself (entity says
    // Vertical, root 0 says Horizontal).
    assert_eq!(
        source.text_attachment_direction,
        TextAttachmentDirectionType::Vertical
    );
    assert_eq!(
        source.context.leader_roots[0].text_attachment_direction,
        TextAttachmentDirectionType::Horizontal
    );

    let imported = import_bytes(&source_bytes);
    let geometries = imported_geometries(&imported);
    let geometry = rich_geometry(&geometries).clone();
    // The model has no entity-level field: only root 0's value is retained.
    assert_eq!(geometry.reserved().roots[0].text_attachment_direction, 0);

    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(geometry)));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);
    let roots = leader_roots(body);
    // Declared behaviour, not fidelity: the source entity-level Vertical is
    // normalised away; the written tail `271` is root 0's Horizontal.
    assert_eq!(value(tail(body), 271), Some("0"));
    assert_eq!(values(&roots[0], 271), vec!["0"]);
    assert_eq!(values(&roots[1], 271), vec!["1"]);
}

/// Declared content-type normalisation: `has_text_contents = true` with an empty
/// string imports as **no content**, so the export writes `172 = 0`.
///
/// The exporter's "empty text content is written as MText content" reason is
/// unreachable from the import path because the import has already normalised
/// the content to `None`.
#[test]
fn empty_text_content_normalises_to_no_content() {
    let source_bytes = adversarial_source_bytes();
    let source_document = read_dxf(&source_bytes);
    let leaders = acad_multileaders(&source_document);
    let source = empty_text_source(&leaders);
    // Fixture sanity: the source claims MTEXT content with an empty string, and
    // its style seed default text is empty.
    assert_eq!(source.content_type, LeaderContentType::MText);
    assert!(source.context.has_text_contents);
    assert!(source.context.text_string.is_empty());
    let style = source_document
        .objects
        .get(&source.style_handle.expect("style handle"))
        .and_then(|object| match object {
            AcadObjectType::MultiLeaderStyle(style) => Some(style),
            _ => None,
        })
        .expect("the source style record survives the write");
    assert!(style.default_text.is_empty());

    let imported = import_bytes(&source_bytes);
    let geometries = imported_geometries(&imported);
    let geometry = empty_text_geometry(&geometries).clone();
    assert_eq!(geometry.content(), MLeaderContent::None);
    assert!(geometry.text().string.is_empty());

    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(geometry)));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);
    assert_eq!(
        value(tail(body), 172),
        Some("0"),
        "empty text content is normalised to no content on export"
    );
    // With no content the writer emits no text string group; the only `304`
    // groups are the `LEADER_LINE{` nesting markers.
    assert!(
        !values(body, 304)
            .iter()
            .any(|value| !value.starts_with("LEADER_LINE")),
        "{:?}",
        values(body, 304)
    );
}

/// Declared: the legacy break mirrors (`segment_index`, `break_info_count`) are
/// re-derived from the break records, never written as independent source
/// values. The source mirrors here say 7/99 while the records say 3/4/count 2.
#[test]
fn break_mirrors_are_rederived_from_the_break_records() {
    let geometry = source_model_with_inconsistent_break_mirrors();
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(geometry)));

    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);
    let lines = leader_lines(body);
    assert_eq!(lines.len(), 1);
    // The only `90` groups in the leader line are the break records' segment
    // indices; the bogus mirror 7 and count 99 are not written.
    assert_eq!(values(&lines[0], 90), vec!["3", "4"]);

    let written_document = read_dxf(&bytes);
    let written = acad_multileaders(&written_document);
    let line = &written[0].context.leader_roots[0].lines[0];
    assert_eq!(line.break_infos.len(), 2);
    assert_eq!(line.break_infos[0].segment_index, 3);
    assert_eq!(line.break_infos[1].segment_index, 4);
    // Declared: the mirrors come from the records, so the written file carries
    // the derived count (2) and first segment (3), not 99/7.
    assert_eq!(line.break_info_count, 2);
    assert_eq!(line.segment_index, 3);

    // The re-imported model stores the same derived mirrors.
    let reimported = import_bytes(&bytes);
    let geometries = imported_geometries(&reimported);
    let line = &geometries[0].reserved().lines[0];
    assert_eq!(line.break_info_count, 2);
    assert_eq!(line.segment_index, 3);
    assert_eq!(line.breaks.len(), 2);
}

/// End-to-end with the declared normalisations: an adversarial source file is
/// imported, exported, and read back with **acadrust's own reader**.
///
/// The effective values that the model keeps must survive; the three declared
/// normalisations must be visible. Byte equality is deliberately not asserted
/// for data the model never stores.
#[test]
fn adversarial_round_trip_keeps_effective_fields_and_pins_normalisations() {
    let source_bytes = adversarial_source_bytes();
    let imported = import_bytes(&source_bytes);
    let geometries = imported_geometries(&imported);
    let rich = with_inconsistent_break_mirrors(rich_geometry(&geometries));
    let empty = empty_text_geometry(&geometries).clone();

    let (bytes, _) = export_db(&db_with(vec![
        SemanticGeometry::MultiLeader(Box::new(rich)),
        SemanticGeometry::MultiLeader(Box::new(empty)),
    ]));
    let written_document = read_dxf(&bytes);
    let written = acad_multileaders(&written_document);
    let rich_written = written
        .iter()
        .find(|leader| leader.context.leader_roots.len() == 2)
        .expect("the rich entity is written");
    let empty_written = written
        .iter()
        .find(|leader| leader.context.leader_roots.len() == 1)
        .expect("the empty-text entity is written");

    // ── Values that must survive ──
    assert_eq!(
        rich_written.property_override_flags.bits(),
        ADVERSARIAL_MASK_BITS
    );
    let roots = &rich_written.context.leader_roots;
    assert_eq!(roots[0].leader_index, 0);
    assert!(approx_vector(
        roots[0].connection_point,
        Vector3::new(10.0, 2.0, 0.0)
    ));
    assert!(approx_vector(
        roots[0].direction,
        Vector3::new(1.0, 0.0, 0.0)
    ));
    assert!(approx(roots[0].landing_distance, 0.3));
    assert_eq!(roots[1].leader_index, 1);
    assert!(approx_vector(
        roots[1].connection_point,
        Vector3::new(-4.0, 2.0, 0.0)
    ));
    assert!(approx_vector(
        roots[1].direction,
        Vector3::new(-1.0, 0.0, 0.0)
    ));
    assert!(approx(roots[1].landing_distance, 0.25));

    assert_eq!(roots[0].lines.len(), 2);
    assert_eq!(roots[1].lines.len(), 1);
    let first = &roots[0].lines[0];
    assert_eq!(first.index, 0);
    assert_eq!(first.points.len(), 3);
    assert!(approx_vector(first.points[0], Vector3::new(0.0, 0.0, 0.0)));
    assert!(approx_vector(first.points[2], Vector3::new(9.0, 1.0, 0.0)));
    assert_eq!(first.override_flags.bits(), MLeaderLine::PATH_TYPE);
    assert_eq!(first.path_type, AcadPathType::StraightLineSegments);
    assert!(approx(first.arrowhead_size, 0.35));
    let spline = &roots[0].lines[1];
    assert_eq!(spline.index, 1);
    assert_eq!(spline.override_flags.bits(), MLeaderLine::ARROWHEAD_SIZE);
    assert_eq!(spline.path_type, AcadPathType::Spline);
    assert!(approx(spline.arrowhead_size, 0.4));
    assert_eq!(spline.arrowhead_handle, Some(AcadHandle::new(0x2A)));
    let second_root_line = &roots[1].lines[0];
    assert_eq!(second_root_line.index, 0);
    assert_eq!(second_root_line.override_flags.bits(), 0);
    assert!(approx(second_root_line.arrowhead_size, 0.2));

    let context = &rich_written.context;
    assert_eq!(context.text_string, "Adversarial");
    assert!(approx_vector(
        context.text_location,
        Vector3::new(10.0, 2.0, 0.0)
    ));
    assert!(approx(context.text_rotation, 0.25));
    assert!(approx(context.text_height, 0.25));
    assert!(approx(context.text_width, 1.5));
    assert_eq!(
        rich_written.text_attachment_point,
        TextAttachmentPointType::Right
    );
    assert_eq!(
        rich_written.text_left_attachment,
        TextAttachmentType::BottomOfBottomLine
    );
    assert!(approx_vector(
        context.base_point,
        Vector3::new(1.0, 2.0, 0.0)
    ));
    assert!(approx_vector(
        context.base_direction,
        Vector3::new(0.0, 1.0, 0.0)
    ));
    assert!(approx_vector(
        context.base_vertical,
        Vector3::new(-1.0, 0.0, 0.0)
    ));
    assert!(context.normal_reversed);

    // ── Declared normalisation 1: entity-tail 271 from root 0 ──
    // The source entity said Vertical; root 0 said Horizontal. The written
    // entity-level direction is root 0's, not the source's.
    assert_eq!(
        rich_written.text_attachment_direction,
        TextAttachmentDirectionType::Horizontal
    );
    assert_eq!(
        roots[0].text_attachment_direction,
        TextAttachmentDirectionType::Horizontal
    );
    assert_eq!(
        roots[1].text_attachment_direction,
        TextAttachmentDirectionType::Vertical
    );

    // ── Declared normalisation 2: empty text is no content ──
    // The source entity said MTEXT with `has_text_contents = true` and an empty
    // string; the import normalises it to no content, so the written entity has
    // no content type and no text contents.
    assert_eq!(empty_written.content_type, LeaderContentType::None);
    assert!(!empty_written.context.has_text_contents);
    assert!(empty_written.context.text_string.is_empty());

    // ── Declared normalisation 3: break mirrors re-derived ──
    // The exporter input carried bogus mirrors 7/99; the written line carries
    // the values derived from its two break records (first segment 3, count 2).
    let break_line = &roots[0].lines[0];
    assert_eq!(break_line.break_infos.len(), 2);
    assert_eq!(break_line.break_info_count, 2);
    assert_eq!(break_line.segment_index, 3);
}
