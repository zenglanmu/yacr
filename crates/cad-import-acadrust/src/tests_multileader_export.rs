//! MULTILEADER export tests (P7b-2, ADR 0019 decision E).
//!
//! The writer emits a real `MULTILEADER` entity, one `MLEADERSTYLE` object per
//! distinct style name and the two `CLASSES` entries. These tests assert the
//! written bytes (mask verbatim, tail codes, nesting, class strings) and the
//! **effective** round trip through acadrust's reader and our importer.
//!
//! Round-trip scope: masked-out raw values are **not** asserted to be
//! byte-identical, because P7b-1 stores only the resolved effective snapshot
//! (deviation 1). The style record written here is that snapshot, so a reload
//! reproduces the effective values even though the raw source record is not
//! reconstructed.

use std::io::Cursor;

use acadrust::{DxfReader, EntityType};

use super::*;
use crate::tests_multileader_fixtures::*;

// ---------------------------------------------------------------------------
// Byte-level DXF helpers.
// ---------------------------------------------------------------------------

/// Parse the ASCII DXF into `(group code, value)` pairs.
fn pairs(bytes: &[u8]) -> Vec<(i32, String)> {
    let text = String::from_utf8(bytes.to_vec()).expect("ASCII DXF");
    let mut lines = text.split("\r\n");
    let mut out = Vec::new();
    while let Some(code) = lines.next() {
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let Ok(code) = code.parse::<i32>() else {
            continue;
        };
        match lines.next() {
            Some(value) => out.push((code, value.to_string())),
            None => break,
        }
    }
    out
}

/// Group the pairs into records at every `0` marker.
fn records(pairs: &[(i32, String)]) -> Vec<(String, Vec<(i32, String)>)> {
    let mut out: Vec<(String, Vec<(i32, String)>)> = Vec::new();
    for (code, value) in pairs {
        if *code == 0 {
            out.push((value.clone(), Vec::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push((*code, value.clone()));
        }
    }
    out
}

fn bodies<'a>(
    records: &'a [(String, Vec<(i32, String)>)],
    name: &str,
) -> Vec<&'a Vec<(i32, String)>> {
    records
        .iter()
        .filter(|(record, _)| record == name)
        .map(|(_, body)| body)
        .collect()
}

fn value(body: &[(i32, String)], code: i32) -> Option<&str> {
    body.iter()
        .find(|(candidate, _)| *candidate == code)
        // Numeric groups are right-aligned in a six-character field; string
        // groups are not padded, so trimming only removes field padding.
        .map(|(_, value)| value.trim())
}

fn values(body: &[(i32, String)], code: i32) -> Vec<&str> {
    body.iter()
        .filter(|(candidate, _)| *candidate == code)
        .map(|(_, value)| value.trim())
        .collect()
}

/// The entity tail: everything after the `CONTEXT_DATA` closing marker.
fn tail(body: &[(i32, String)]) -> &[(i32, String)] {
    let end = body
        .iter()
        .position(|pair| pair == &(301, "}".to_string()))
        .expect("CONTEXT_DATA is closed");
    &body[end + 1..]
}

fn hex(body: &[(i32, String)], code: i32) -> u64 {
    u64::from_str_radix(value(body, code).expect("handle group"), 16).expect("hex handle")
}

/// The nested `LEADER_LINE{ … }` bodies, in written order.
fn leader_lines(body: &[(i32, String)]) -> Vec<Vec<(i32, String)>> {
    let mut out = Vec::new();
    let mut current: Option<Vec<(i32, String)>> = None;
    for pair in body {
        if pair.0 == 304 && pair.1 == "LEADER_LINE{" {
            current = Some(Vec::new());
        } else if pair.0 == 305 {
            if let Some(line) = current.take() {
                out.push(line);
            }
        } else if let Some(line) = current.as_mut() {
            line.push(pair.clone());
        }
    }
    out
}

/// The nested `LEADER{ … }` bodies (leader-line content excluded), in order.
fn leader_roots(body: &[(i32, String)]) -> Vec<Vec<(i32, String)>> {
    let mut out = Vec::new();
    let mut current: Option<Vec<(i32, String)>> = None;
    let mut in_line = false;
    for pair in body {
        if pair.0 == 302 && pair.1 == "LEADER{" {
            current = Some(Vec::new());
        } else if pair.0 == 303 {
            if let Some(root) = current.take() {
                out.push(root);
            }
        } else if pair.0 == 304 && pair.1 == "LEADER_LINE{" {
            in_line = true;
        } else if pair.0 == 305 {
            in_line = false;
        } else if !in_line {
            if let Some(root) = current.as_mut() {
                root.push(pair.clone());
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Byte assertions.
// ---------------------------------------------------------------------------

#[test]
fn first_class_multileader_kind_name_is_multileader() {
    let geometry = SemanticGeometry::MultiLeader(Box::new(rich_multileader()));
    assert_eq!(crate::export::kind_name(&geometry), "MultiLeader");
}

#[test]
fn multileader_is_written_as_a_real_multileader_entity() {
    let (bytes, _report) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let text = String::from_utf8(bytes.clone()).expect("ASCII DXF");
    assert!(text.contains("\r\nMULTILEADER\r\n"), "entity type missing");

    let decoded = pairs(&bytes);
    let all = records(&decoded);
    let multileaders = bodies(&all, "MULTILEADER");
    assert_eq!(multileaders.len(), 1, "exactly one MULTILEADER entity");
    // The derived display is never written as separate primitives.
    for kind in ["LINE", "LWPOLYLINE", "SPLINE", "MTEXT", "TEXT"] {
        assert!(
            bodies(&all, kind).is_empty(),
            "{kind} must not be emitted for a MULTILEADER"
        );
    }
}

#[test]
fn the_override_mask_and_style_handle_are_written_verbatim_in_the_tail() {
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);
    let entity_tail = tail(body);

    // The entity tail starts with the style handle, then the raw `90` mask.
    assert_eq!(entity_tail[0].0, 340, "{entity_tail:?}");
    let style_handle = u64::from_str_radix(entity_tail[0].1.trim(), 16).expect("hex handle");
    assert_eq!(
        (entity_tail[1].0, entity_tail[1].1.trim()),
        (90, "66064"),
        "the stored mask 0x0001_0210 must be written verbatim"
    );
    assert_eq!(
        (entity_tail[2].0, entity_tail[2].1.trim()),
        (170, "1"),
        "leader path type"
    );
    assert_eq!(value(entity_tail, 172), Some("2"), "content type is MText");
    assert_eq!(
        value(entity_tail, 45),
        Some("2.0"),
        "tail 45 is the scale factor"
    );

    // `340` points at an emitted MLEADERSTYLE record.
    let styles = bodies(&all, "MLEADERSTYLE");
    let target = styles
        .iter()
        .find(|style| hex(style, 5) == style_handle)
        .expect("the entity's style handle resolves to an emitted record");
    assert_eq!(value(target, 3), Some("Standard"));

    // A zero mask is still written (never omitted).
    let (zero_bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(multileader(
        "Standard", 0, 0.05,
    ))));
    let zero_records = records(&pairs(&zero_bytes));
    let zero_body = bodies(&zero_records, "MULTILEADER").remove(0);
    let zero_tail = tail(zero_body);
    assert_eq!(
        (zero_tail[1].0, zero_tail[1].1.trim()),
        (90, "0"),
        "a zero mask is written as 0, not omitted"
    );
}

#[test]
fn leader_roots_nest_their_lines_and_write_every_vertex() {
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);

    assert_eq!(values(body, 302), vec!["LEADER{", "LEADER{"]);
    let line_markers: Vec<&str> = values(body, 304)
        .into_iter()
        .filter(|value| value.starts_with("LEADER_LINE"))
        .collect();
    assert_eq!(
        line_markers,
        vec!["LEADER_LINE{", "LEADER_LINE{", "LEADER_LINE{"]
    );
    assert_eq!(values(body, 303), vec!["}", "}"]);
    assert_eq!(values(body, 305), vec!["}", "}", "}"]);

    // One `10/20/30` triple per input vertex, per line, in order.
    let lines = leader_lines(body);
    assert_eq!(lines.len(), 3);
    let xs: Vec<Vec<&str>> = lines.iter().map(|line| values(line, 10)).collect();
    assert_eq!(xs[0], vec!["0.0", "5.0", "9.0"]);
    assert_eq!(xs[1], vec!["0.0", "5.0"]);
    assert_eq!(xs[2], vec!["-6.0", "-5.5", "-5.0"]);
    for line in &lines {
        assert_eq!(values(line, 10).len(), values(line, 20).len());
        assert_eq!(values(line, 10).len(), values(line, 30).len());
    }

    // Per-root `leader_index` (90) and dogleg length (40).
    let roots = leader_roots(body);
    assert_eq!(roots.len(), 2);
    assert_eq!(values(&roots[0], 90), vec!["0"]);
    assert_eq!(values(&roots[1], 90), vec!["1"]);
    assert_eq!(values(&roots[0], 40), vec!["0.3"]);
    assert_eq!(values(&roots[1], 40), vec!["0.25"]);
    // The root break pair is written as 12/13, once.
    assert_eq!(values(&roots[0], 12), vec!["1.0"]);
    assert_eq!(values(&roots[0], 13), vec!["2.0"]);
    assert!(values(&roots[1], 12).is_empty());
}

#[test]
fn per_line_override_flags_and_arrowhead_fields_are_written() {
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);

    // 91 = line index, 93 = the verbatim per-line override word, 40 = the
    // per-line arrowhead size.
    let lines = leader_lines(body);
    assert_eq!(lines.len(), 3);
    assert_eq!(values(&lines[0], 91), vec!["0"]);
    assert_eq!(values(&lines[1], 91), vec!["1"]);
    assert_eq!(values(&lines[2], 91), vec!["0"]);
    assert_eq!(values(&lines[0], 93), vec!["1"]);
    assert_eq!(values(&lines[1], 93), vec!["16"]);
    assert_eq!(values(&lines[2], 93), vec!["0"]);
    assert_eq!(values(&lines[0], 40), vec!["0.35"]);
    assert_eq!(values(&lines[1], 40), vec!["0.4"]);
    assert_eq!(values(&lines[2], 40), vec!["0.2"]);

    // The second line carries the custom arrowhead handle (341); the others
    // write the null handle acadrust always emits.
    assert_eq!(values(&lines[1], 341), vec!["2A"]);
    assert_eq!(values(&lines[0], 341), vec!["0"]);
    // The first line's break segment index (90) is written once.
    assert_eq!(values(&lines[0], 90), vec!["1"]);
    assert!(values(&lines[1], 90).is_empty());

    // The entity tail's own per-line defaults: 91 colour, 341 linetype, 171 weight.
    let tail = tail(body);
    assert!(value(tail, 91).is_some());
    assert!(value(tail, 341).is_some());
    assert!(value(tail, 171).is_some());
}

#[test]
fn attachment_direction_is_written_where_acadrust_writes_it() {
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);

    // 271: once per LEADER root (root 0 = vertical, root 1 = horizontal) and
    // once in the entity tail (from root 0). 272/273: in CONTEXT_DATA and in
    // the tail — acadrust writes both, so this pins that we do not suppress
    // either occurrence.
    let roots = leader_roots(body);
    assert_eq!(values(&roots[0], 271), vec!["1"]);
    assert_eq!(values(&roots[1], 271), vec!["0"]);
    assert_eq!(value(tail(body), 271), Some("1"));
    assert_eq!(values(body, 272), vec!["9", "9"]);
    assert_eq!(values(body, 273), vec!["9", "9"]);
}

#[test]
fn classes_entries_carry_the_exact_names_and_flags() {
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let text = String::from_utf8(bytes.clone()).expect("ASCII DXF");
    // The literal strings, not a rendered equivalent.
    for literal in [
        "\r\nMULTILEADER\r\n",
        "\r\nAcDbMLeader\r\n",
        "\r\nACDB_MLEADER_CLASS\r\n",
        "\r\nMLEADERSTYLE\r\n",
        "\r\nAcDbMLeaderStyle\r\n",
        "\r\nACDB_MLEADERSTYLE_CLASS\r\n",
    ] {
        assert!(text.contains(literal), "missing literal {literal:?}");
    }

    let all = records(&pairs(&bytes));
    let classes = bodies(&all, "CLASS");
    assert!(
        !classes.is_empty(),
        "acadrust writes its default class table"
    );

    let entity_class = classes
        .iter()
        .find(|class| value(class, 1) == Some("MULTILEADER"))
        .expect("entity class present");
    assert_eq!(value(entity_class, 2), Some("AcDbMLeader"));
    assert_eq!(value(entity_class, 3), Some("ACDB_MLEADER_CLASS"));
    assert_eq!(value(entity_class, 90), Some("1025"), "proxy flags (U-5)");
    assert_eq!(value(entity_class, 91), Some("0"), "instance count (U-5)");
    assert_eq!(value(entity_class, 280), Some("0"), "was-zombie (U-5)");
    assert_eq!(value(entity_class, 281), Some("1"), "is-an-entity");

    let style_class = classes
        .iter()
        .find(|class| value(class, 1) == Some("MLEADERSTYLE"))
        .expect("style class present");
    assert_eq!(value(style_class, 2), Some("AcDbMLeaderStyle"));
    assert_eq!(value(style_class, 3), Some("ACDB_MLEADERSTYLE_CLASS"));
    assert_eq!(value(style_class, 90), Some("4095"));
    assert_eq!(value(style_class, 91), Some("0"));
    assert_eq!(value(style_class, 280), Some("0"));
    assert_eq!(value(style_class, 281), Some("0"), "not an entity");

    // Exactly one entry per class, and group 1 is never `MLEADER` (a real file
    // registers the entity class as `MULTILEADER`).
    for name in ["MULTILEADER", "MLEADERSTYLE"] {
        assert_eq!(
            classes
                .iter()
                .filter(|class| value(class, 1) == Some(name))
                .count(),
            1,
            "{name}"
        );
    }
    assert!(!classes
        .iter()
        .any(|class| value(class, 1) == Some("MLEADER")));
}

#[test]
fn one_style_record_per_distinct_name_is_shared_by_both_entities() {
    let db = db_with(vec![
        SemanticGeometry::MultiLeader(Box::new(multileader("Test", 0x10, 0.05))),
        SemanticGeometry::MultiLeader(Box::new(multileader("Test", 0x10, 0.05))),
    ]);
    let (bytes, _) = export_db(&db);
    let all = records(&pairs(&bytes));

    let styles = bodies(&all, "MLEADERSTYLE");
    assert_eq!(styles.len(), 2, "Standard plus one deduped 'Test' record");
    let style_position = |wanted: &str| {
        all.iter()
            .position(|(name, body)| name == "MLEADERSTYLE" && value(body, 3) == Some(wanted))
            .unwrap_or_else(|| panic!("{wanted} record"))
    };
    let standard_position = style_position("Standard");
    let test_position = style_position("Test");
    // Standard is emitted first (the document already owns its low handle).
    assert!(
        standard_position < test_position,
        "{standard_position} < {test_position}"
    );

    let test = &all[test_position].1;
    let test_handle = hex(test, 5);
    let emitted: Vec<u64> = bodies(&all, "MULTILEADER")
        .iter()
        .map(|body| hex(tail(body), 340))
        .collect();
    assert_eq!(emitted, vec![test_handle, test_handle]);

    // The record carries the resolved snapshot, not a second source of truth.
    assert_eq!(value(test, 173), Some("1"), "path type");
    assert_eq!(value(test, 43), Some("0.3"), "landing distance");
    assert_eq!(value(test, 42), Some("0.05"), "landing gap");
    assert_eq!(value(test, 44), Some("0.2"), "arrowhead size");
    assert_eq!(value(test, 45), Some("0.25"), "text height");
}

#[test]
fn conflicting_shared_style_names_keep_the_first_and_say_so() {
    let db = db_with(vec![
        SemanticGeometry::MultiLeader(Box::new(multileader("Test", 0x10, 0.05))),
        SemanticGeometry::MultiLeader(Box::new(multileader("Test", 0x10, 0.09))),
    ]);
    let (bytes, report) = export_db(&db);
    let all = records(&pairs(&bytes));
    assert_eq!(bodies(&all, "MLEADERSTYLE").len(), 2);
    assert!(
        report.entries.iter().any(|entry| {
            entry.code == "export.converted.multileader"
                && entry.message.contains("first entity's snapshot")
        }),
        "{:?}",
        report.entries
    );
}

// ---------------------------------------------------------------------------
// Report: named deviations, never silent loss.
// ---------------------------------------------------------------------------

#[test]
fn the_report_names_every_multileader_deviation() {
    let (_, report) = export_one(SemanticGeometry::MultiLeader(Box::new(rich_multileader())));
    let messages: Vec<&str> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.converted.multileader")
        .map(|entry| entry.message.as_str())
        .collect();

    for expected in [
        "91/341/171",
        "90 property-override mask is written verbatim",
        "resolved effective values snapshot",
        "not a verbatim round-trip",
        "seed values",
        "background-fill",
        "column-layout",
        "break data",
        "arrowhead block handle 2A",
        "linetype handles are written verbatim",
    ] {
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "missing '{expected}' in {messages:?}"
        );
    }

    assert_eq!(report.counts.converted, 1);
    assert_eq!(report.counts.dropped, 0);
    assert!(
        matches!(report.completeness, Completeness::Partial(_)),
        "a MULTILEADER export is never Complete: {:?}",
        report.completeness
    );
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code.starts_with("export.dropped.")),
        "{:?}",
        report.entries
    );
}

// ---------------------------------------------------------------------------
// Effective round trip.
// ---------------------------------------------------------------------------

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn approx_point(a: Point3, b: Point3) -> bool {
    approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
}

#[test]
fn multileader_round_trips_its_effective_fields_through_acadrust() {
    let source = rich_multileader();
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(source.clone())));

    // 1. The written file is readable by acadrust and still has one
    //    MULTILEADER with the expected structure.
    let document = DxfReader::from_reader(Cursor::new(bytes.clone()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    let written = document
        .entities()
        .find_map(|entity| match entity {
            EntityType::MultiLeader(multileader) => Some(multileader.as_ref()),
            _ => None,
        })
        .expect("one MULTILEADER entity");
    assert_eq!(written.context.leader_roots.len(), 2);
    assert_eq!(written.total_leader_line_count(), 3);
    assert_eq!(written.property_override_flags.bits(), 0x0001_0210);
    assert_eq!(
        written.content_type,
        acadrust::entities::LeaderContentType::MText
    );
    assert_eq!(written.context.text_string, "Note");
    assert!(
        document.objects.values().any(|object| matches!(
            object,
            acadrust::objects::ObjectType::MultiLeaderStyle(style) if style.name == "Standard"
        )),
        "the style object is readable"
    );

    // 2. Re-import and compare the **effective** fields. Masked-out raw values
    //    are not asserted to be byte-identical: P7b-1 stores the resolved
    //    snapshot, so the source raw record is not reconstructed (deviation 1).
    let reimported = import_bytes(&bytes);
    let geometry = reimported
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(multileader) => Some(multileader.as_ref()),
            _ => None,
        })
        .expect("re-imported MULTILEADER variant");

    assert_eq!(geometry.override_mask(), source.override_mask());
    assert_eq!(geometry.style_name(), source.style_name());
    assert_eq!(geometry.content(), source.content());

    let style = geometry.style();
    let expected_style = source.style();
    assert_eq!(style.path_type, expected_style.path_type);
    assert_eq!(style.enable_landing, expected_style.enable_landing);
    assert_eq!(style.enable_dogleg, expected_style.enable_dogleg);
    assert!(approx(
        style.landing_distance,
        expected_style.landing_distance
    ));
    assert!(approx(style.landing_gap, expected_style.landing_gap));
    assert!(approx(style.arrowhead_size, expected_style.arrowhead_size));
    assert!(approx(style.text_height, expected_style.text_height));
    assert!(approx(style.scale_factor, expected_style.scale_factor));
    assert_eq!(style.text_style_name, expected_style.text_style_name);

    let text = geometry.text();
    let expected_text = source.text();
    assert_eq!(text.string, expected_text.string);
    assert!(approx_point(text.location, expected_text.location));
    assert!(approx(text.rotation, expected_text.rotation));
    assert!(approx(text.height, expected_text.height));
    assert!(approx(text.width, expected_text.width));
    assert_eq!(text.attachment_point, expected_text.attachment_point);
    assert_eq!(text.left_attachment, expected_text.left_attachment);

    assert_eq!(geometry.roots().len(), source.roots().len());
    for (actual, expected) in geometry.roots().iter().zip(source.roots()) {
        assert_eq!(actual.leader_index, expected.leader_index);
        assert!(approx_point(actual.landing_point, expected.landing_point));
        assert!(approx_point(actual.dogleg_vector, expected.dogleg_vector));
        assert!(approx(actual.dogleg_length, expected.dogleg_length));
    }

    assert_eq!(geometry.lines().len(), source.lines().len());
    for (actual, expected) in geometry.lines().iter().zip(source.lines()) {
        assert_eq!(actual.root_index, expected.root_index);
        assert_eq!(actual.index, expected.index);
        assert_eq!(actual.points.len(), expected.points.len());
        for (a, b) in actual.points.iter().zip(&expected.points) {
            assert!(approx_point(*a, *b), "{a:?} != {b:?}");
        }
        assert_eq!(actual.path_type, expected.path_type);
        assert_eq!(actual.override_flags, expected.override_flags);
        assert!(approx(actual.arrowhead_size, expected.arrowhead_size));
        assert_eq!(actual.arrowhead_handle, expected.arrowhead_handle);
    }

    // Preserved-but-unrendered data is written once and comes back verbatim.
    let reserved = geometry.reserved();
    let expected_reserved = source.reserved();
    assert_eq!(reserved.roots, expected_reserved.roots);
    assert_eq!(reserved.lines, expected_reserved.lines);
    assert_eq!(reserved.background, expected_reserved.background);
    assert_eq!(reserved.columns, expected_reserved.columns);

    let plane = geometry.plane();
    let expected_plane = source.plane();
    assert!(approx_point(plane.origin, expected_plane.origin));
    assert!(approx_point(plane.u, expected_plane.u));
    assert!(approx_point(plane.v, expected_plane.v));
    assert_eq!(plane.normal_reversed, expected_plane.normal_reversed);
}

#[test]
fn a_contentless_multileader_writes_172_zero_and_round_trips() {
    let (bytes, _) = export_one(SemanticGeometry::MultiLeader(Box::new(
        contentless_multileader(),
    )));
    let all = records(&pairs(&bytes));
    let body = bodies(&all, "MULTILEADER").remove(0);
    assert_eq!(value(tail(body), 172), Some("0"), "content type None");
    // With no text content there is no string group at all.
    assert!(
        !values(body, 304)
            .iter()
            .any(|value| !value.starts_with("LEADER_LINE")),
        "{:?}",
        values(body, 304)
    );

    let reimported = import_bytes(&bytes);
    let geometry = reimported
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(multileader) => Some(multileader.as_ref()),
            _ => None,
        })
        .expect("contentless MULTILEADER re-imports");
    assert_eq!(geometry.content(), MLeaderContent::None);
    assert_eq!(geometry.override_mask(), 0);
    assert_eq!(geometry.lines().len(), 1);
}

// ---------------------------------------------------------------------------
// No regression.
// ---------------------------------------------------------------------------

#[test]
fn a_compound_still_flattens_and_never_becomes_a_multileader() {
    let (bytes, report) = export_one(SemanticGeometry::Compound(vec![
        SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        SemanticGeometry::Circle {
            center: p(1.0, 1.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 0.5,
        },
    ]));
    let all = records(&pairs(&bytes));
    assert!(bodies(&all, "MULTILEADER").is_empty());
    assert_eq!(bodies(&all, "LINE").len(), 1);
    assert_eq!(bodies(&all, "CIRCLE").len(), 1);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.compound"),
        "{:?}",
        report.entries
    );
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.multileader"),
        "{:?}",
        report.entries
    );
}

// ---------------------------------------------------------------------------
// Insert placement: rigid stays an entity, scale/mirror flattens.
// ---------------------------------------------------------------------------

/// A rigid placement (translation here) is still baked into a real
/// `MULTILEADER`, never flattened.
#[test]
fn a_rigidly_placed_multileader_stays_a_real_multileader() {
    let db = db_with_block(
        SemanticGeometry::MultiLeader(Box::new(straight_multileader())),
        Transform3::translation(p(5.0, 3.0, 0.0)),
    );
    let (bytes, report) = export_db(&db);
    let all = records(&pairs(&bytes));
    assert_eq!(bodies(&all, "MULTILEADER").len(), 1);
    assert!(bodies(&all, "LWPOLYLINE").is_empty());
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.multileader_flattened"),
        "{:?}",
        report.entries
    );

    let reimported = import_bytes(&bytes);
    let geometry = reimported
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(multileader) => Some(multileader.as_ref()),
            _ => None,
        })
        .expect("the rigid placement keeps the MULTILEADER");
    assert_eq!(geometry.roots()[0].landing_point, p(15.0, 5.0, 0.0));
    assert_eq!(geometry.lines()[0].points[0], p(5.0, 3.0, 0.0));
}

/// A scaling insert cannot be baked into the entity (decision C.5), so the
/// multileader survives as its display primitives — the same conversion the
/// pre-P7b `Compound` path gave it — rather than vanishing.
#[test]
fn a_scaled_insert_placement_flattens_the_multileader_instead_of_dropping_it() {
    let db = db_with_block(
        SemanticGeometry::MultiLeader(Box::new(straight_multileader())),
        Transform3::scale(2.0),
    );
    let (bytes, report) = export_db(&db);
    let all = records(&pairs(&bytes));
    let kinds: Vec<&str> = all.iter().map(|(name, _)| name.as_str()).collect();
    assert!(!kinds.contains(&"MULTILEADER"), "{kinds:?}");

    // Display: leader polyline (3 vertices), arrowhead mesh, dogleg line, text.
    let polylines = bodies(&all, "LWPOLYLINE");
    assert_eq!(polylines.len(), 1, "{kinds:?}");
    assert_eq!(values(polylines[0], 10), vec!["0.0", "10.0", "18.0"]);
    // The arrowhead mesh is a polyface mesh: `POLYLINE` + VERTEX records
    // (three vertices and one face record), not a record named `POLYFACE`.
    assert_eq!(bodies(&all, "POLYLINE").len(), 1, "{kinds:?}");
    assert_eq!(bodies(&all, "VERTEX").len(), 4, "{kinds:?}");
    assert_eq!(bodies(&all, "LINE").len(), 1, "{kinds:?}");
    let texts = bodies(&all, "TEXT");
    assert_eq!(texts.len(), 1, "{kinds:?}");
    assert_eq!(value(texts[0], 40), Some("0.36"), "text height is scaled");

    assert!(
        report.entries.iter().any(|entry| {
            entry.code == "export.converted.multileader_flattened"
                && entry
                    .message
                    .contains("flattened to its display primitives")
                && entry
                    .message
                    .contains("no longer a MULTILEADER on round trip")
        }),
        "{:?}",
        report.entries
    );
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code == "export.dropped.multileader"),
        "{:?}",
        report.entries
    );
}

/// A mirrored insert also flattens. **Observed**: the leader polyline,
/// arrowhead mesh and dogleg line are written (they accept a mirror), while the
/// `Text` child is refused by the domain's mirror guard and recorded as that
/// child's own `export.dropped.transform` outcome — the refusal is not
/// swallowed and the parent is still not a drop.
#[test]
fn a_mirrored_insert_placement_flattens_and_records_the_text_refusal() {
    // A true reflection (determinant -1): x -> 10 - x. `Transform3::from_basis`
    // derives a right-handed Z column, so it cannot express this one.
    let mut mirror = Transform3::identity().matrix;
    mirror[0][0] = -1.0;
    mirror[0][3] = 10.0;
    let db = db_with_block(
        SemanticGeometry::MultiLeader(Box::new(straight_multileader())),
        Transform3 { matrix: mirror },
    );
    let (bytes, report) = export_db(&db);
    let all = records(&pairs(&bytes));
    let kinds: Vec<&str> = all.iter().map(|(name, _)| name.as_str()).collect();
    assert!(!kinds.contains(&"MULTILEADER"), "{kinds:?}");

    let polylines = bodies(&all, "LWPOLYLINE");
    assert_eq!(polylines.len(), 1, "{kinds:?}");
    // x -> 10 - x: (0,0) -> (10,0), (5,0) -> (5,0), (9,1) -> (1,1).
    assert_eq!(values(polylines[0], 10), vec!["10.0", "5.0", "1.0"]);
    assert_eq!(bodies(&all, "POLYLINE").len(), 1, "{kinds:?}");
    assert_eq!(bodies(&all, "VERTEX").len(), 4, "{kinds:?}");
    assert_eq!(bodies(&all, "LINE").len(), 1, "{kinds:?}");
    // The flat-text domain cannot represent reflected glyphs (ADR 0019 B).
    assert!(bodies(&all, "TEXT").is_empty(), "{kinds:?}");

    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.multileader_flattened"),
        "{:?}",
        report.entries
    );
    assert!(
        report.entries.iter().any(|entry| {
            entry.code == "export.dropped.transform" && entry.message.contains("Text")
        }),
        "the text child's own refusal must be recorded: {:?}",
        report.entries
    );
}
