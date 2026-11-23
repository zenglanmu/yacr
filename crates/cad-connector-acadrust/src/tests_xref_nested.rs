//! Nested/cyclic/spliced external-reference contracts.
//!
//! Split from `tests_xref` to keep each test file inside the file-size gate.

use super::*;

// ---------------------------------------------------------------------------
// Cycles, overlays and nesting
// ---------------------------------------------------------------------------

#[test]
fn cycle_settles_as_cycle_and_terminates() {
    let mut a = acadrust::CadDocument::new();
    add_xref_block(
        &mut a,
        "B",
        XrefSpec {
            path: "b.dxf",
            ..XrefSpec::default()
        },
    );
    add_model_insert(&mut a, "B", acadrust::types::Vector3::ZERO);

    let mut b = acadrust::CadDocument::new();
    add_xref_block(
        &mut b,
        "A",
        XrefSpec {
            path: "a.dxf",
            ..XrefSpec::default()
        },
    );
    add_model_insert(&mut b, "A", acadrust::types::Vector3::ZERO);

    let host = host_with_xref(
        "A",
        XrefSpec {
            path: "a.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::new(&[
        ("a.dxf", dxf_bytes(&a)),
        ("b.dxf", dxf_bytes(&b)),
    ]));
    let drawing = run_import(&host, Some(Arc::new(options(source)))).expect("terminates");

    assert_eq!(
        reference(&drawing, "A").status,
        ExternalReferenceStatus::Loaded
    );
    let cycle = drawing
        .database
        .external_references()
        .find(|reference| reference.status == ExternalReferenceStatus::Cycle)
        .expect("the nested cycle is recorded");
    assert_eq!(cycle.raw_path, "a.dxf");
    assert!(has_diagnostic(&drawing, "import.xref_cycle"));
}

#[test]
fn nested_overlay_is_skipped_but_top_level_overlay_loads() {
    // outer is a top-level overlay attachment: it still loads.
    let host = host_with_xref(
        "outer",
        XrefSpec {
            path: "outer.dxf",
            overlay: true,
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    // Inside outer, `inner` is an overlay: it is skipped because it is nested.
    let mut outer = acadrust::CadDocument::new();
    add_xref_block(
        &mut outer,
        "inner",
        XrefSpec {
            path: "inner.dxf",
            overlay: true,
            ..XrefSpec::default()
        },
    );
    add_model_insert(&mut outer, "inner", acadrust::types::Vector3::ZERO);
    let inner = reference_line_doc("0", 0.0);

    let source = Arc::new(MemorySource::new(&[
        ("outer.dxf", dxf_bytes(&outer)),
        ("inner.dxf", dxf_bytes(&inner)),
    ]));
    let drawing =
        run_import(&host, Some(Arc::new(options(Arc::clone(&source))))).expect("import succeeds");
    assert_eq!(
        reference(&drawing, "outer").status,
        ExternalReferenceStatus::Loaded
    );
    let skipped = drawing
        .database
        .external_references()
        .find(|reference| reference.status == ExternalReferenceStatus::SkippedOverlay)
        .expect("nested overlay is recorded as skipped");
    assert_eq!(skipped.raw_path, "inner.dxf");
    assert!(has_diagnostic(&drawing, "import.xref_overlay_skipped"));
    // The overlay file was never read: only outer.dxf was requested.
    assert_eq!(source.read_count(), 1);
}

#[test]
fn nested_missing_xref_weakens_the_host_insert_through_the_fixpoint() {
    let host = host_with_xref(
        "outer",
        XrefSpec {
            path: "outer.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    // `outer` loads, but its own attachment is missing.
    let mut outer = acadrust::CadDocument::new();
    add_xref_block(
        &mut outer,
        "inner",
        XrefSpec {
            path: "inner.dxf",
            ..XrefSpec::default()
        },
    );
    add_model_insert(&mut outer, "inner", acadrust::types::Vector3::ZERO);
    let source = Arc::new(MemorySource::new(&[("outer.dxf", dxf_bytes(&outer))]));
    let drawing = run_import(&host, Some(Arc::new(options(source)))).expect("import succeeds");

    assert_eq!(
        reference(&drawing, "outer").status,
        ExternalReferenceStatus::Loaded
    );
    let inner = drawing
        .database
        .external_references()
        .find(|reference| reference.raw_path == "inner.dxf")
        .expect("nested missing reference is recorded");
    assert_eq!(inner.status, ExternalReferenceStatus::Missing);

    let capability = drawing
        .report
        .capabilities
        .iter()
        .find(|capability| capability.type_key == "AcDbBlockReference")
        .expect("the host INSERT capability row exists");
    assert_ne!(
        capability.semantic,
        SupportStatus::Verified,
        "the nested loss must reach the host INSERT's capability"
    );
    assert!(
        drawing
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "import.xref_insert_missing"
                && diagnostic.message.contains("inner.dxf")),
        "{:?}",
        drawing.report.diagnostics
    );
    assert!(!matches!(
        drawing.report.completeness,
        Completeness::Complete
    ));
}

#[test]
fn nested_layers_compose_the_reference_chain() {
    let mut outer = acadrust::CadDocument::new();
    add_layer(&mut outer, "layer", acadrust::Color::WHITE);
    add_model_line(
        &mut outer,
        "layer",
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(1.0, 0.0, 0.0),
    );
    add_xref_block(
        &mut outer,
        "inner",
        XrefSpec {
            path: "inner.dxf",
            ..XrefSpec::default()
        },
    );
    add_model_insert(
        &mut outer,
        "inner",
        acadrust::types::Vector3::new(0.0, 5.0, 0.0),
    );
    let inner = reference_line_doc("layer", 0.0);

    let host = host_with_xref(
        "outer",
        XrefSpec {
            path: "outer.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::new(20.0, 0.0, 0.0),
    );
    let source = Arc::new(MemorySource::new(&[
        ("outer.dxf", dxf_bytes(&outer)),
        ("inner.dxf", dxf_bytes(&inner)),
    ]));
    let drawing = run_import(&host, Some(Arc::new(options(source)))).expect("import succeeds");

    let names: Vec<String> = drawing
        .database
        .layers()
        .map(|layer| layer.name.clone())
        .collect();
    assert!(names.iter().any(|name| name == "outer|layer"), "{names:?}");
    assert!(
        names.iter().any(|name| name == "outer|inner|layer"),
        "{names:?}"
    );
}

#[test]
fn anonymous_referenced_block_does_not_collide_with_the_host_block() {
    let mut host = acadrust::CadDocument::new();
    // Host's own anonymous block `*D0`.
    let host_anon = add_block(&mut host, "*D0");
    add_block_line(&mut host, host_anon, "0", 5.0);
    // And the xref attachment.
    add_xref_block(
        &mut host,
        "XREF",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
    );
    add_model_insert(&mut host, "XREF", acadrust::types::Vector3::ZERO);

    let mut referenced = acadrust::CadDocument::new();
    let referenced_anon = add_block(&mut referenced, "*D0");
    add_block_line(&mut referenced, referenced_anon, "0", 7.0);
    add_model_insert(&mut referenced, "*D0", acadrust::types::Vector3::ZERO);

    let source = Arc::new(MemorySource::new(&[("base.dxf", dxf_bytes(&referenced))]));
    let drawing = run_import(&host, Some(Arc::new(options(source)))).expect("import succeeds");

    // Host's `*D0` still holds its own line at y = 5.
    let host_block = block_named(&drawing, "*D0");
    let host_member = entity(&drawing, host_block.entities[0]);
    assert!(matches!(
        host_member.geometry,
        SemanticGeometry::Line { start, .. } if (start.y - 5.0).abs() < 1e-9
    ));

    // The referenced block was namespaced instead of overwriting it.
    let namespaced = block_named(&drawing, "XREF$0$*D0");
    let namespaced_member = entity(&drawing, namespaced.entities[0]);
    assert!(matches!(
        namespaced_member.geometry,
        SemanticGeometry::Line { start, .. } if (start.y - 7.0).abs() < 1e-9
    ));
}

// ---------------------------------------------------------------------------
// Spliced display trees and dropped referenced content
// ---------------------------------------------------------------------------

fn dimension_style() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: 0.0,
        dim_line_increment: 3.75,
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

fn picture_dimension(display: Vec<SemanticGeometry>) -> DimensionGeometry {
    DimensionGeometry::from_picture(
        DimensionDefinition {
            kind: DimensionKind::Aligned,
            points: DimensionPoints::Linear {
                first: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                second: Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
                dimension_line: Point3 {
                    x: 0.0,
                    y: -5.0,
                    z: 0.0,
                },
            },
            text_middle: Point3::default(),
            text_rotation: 0.0,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        },
        "Standard".to_string(),
        dimension_style(),
        None,
        display,
    )
    .expect("picture dimension")
}

#[test]
fn referenced_dimension_display_insert_is_remapped_instead_of_aliasing_a_host_block() {
    // A referenced dimension picture block that INSERTs a referenced block.
    // The referenced id happens to equal a host id (ids alias across dbs).
    let referenced_block = BlockId(3);
    let host_block_with_same_id = BlockId(3);
    let geometry = SemanticGeometry::Dimension(Box::new(picture_dimension(vec![
        SemanticGeometry::Insert {
            block: referenced_block,
            transform: Transform3::default(),
        },
    ])));
    let mut block_map = HashMap::new();
    block_map.insert(referenced_block, BlockId(40));
    let mut missing = Vec::new();

    let remapped = crate::xref::remap_geometry(&geometry, &block_map, &mut missing);
    let SemanticGeometry::Dimension(dimension) = &remapped else {
        panic!("expected a dimension, got {remapped:?}");
    };
    let SemanticGeometry::Insert { block, .. } = &dimension.display()[0] else {
        panic!("expected the display INSERT, got {:?}", dimension.display());
    };
    assert_eq!(*block, BlockId(40));
    assert_ne!(*block, host_block_with_same_id);
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn unmapped_referenced_block_uses_the_sentinel_and_reports_it() {
    let referenced_block = BlockId(9);
    let geometry = SemanticGeometry::Insert {
        block: referenced_block,
        transform: Transform3::default(),
    };
    let mut missing = Vec::new();
    let remapped = crate::xref::remap_geometry(&geometry, &HashMap::new(), &mut missing);
    let SemanticGeometry::Insert { block, .. } = remapped else {
        panic!("expected an INSERT");
    };
    assert_eq!(block, crate::xref::UNMAPPED_BLOCK);
    assert_ne!(
        block, referenced_block,
        "a miss must not alias the source id"
    );
    assert_eq!(missing, vec![referenced_block]);
}

#[test]
fn referenced_paper_space_is_reported_as_skipped() {
    let mut referenced = reference_line_doc("0", 0.0);
    let mut paper_line = acadrust::entities::Line::new();
    paper_line.start = acadrust::types::Vector3::new(0.0, 40.0, 0.0);
    paper_line.end = acadrust::types::Vector3::new(1.0, 40.0, 0.0);
    paper_line.common.layer = "0".to_string();
    referenced
        .add_paper_space_entity(EntityType::Line(paper_line))
        .expect("add paper-space line");

    let document = host_with_xref(
        "XREF",
        XrefSpec {
            path: "base.dxf",
            ..XrefSpec::default()
        },
        acadrust::types::Vector3::ZERO,
    );
    let source = Arc::new(MemorySource::new(&[("base.dxf", dxf_bytes(&referenced))]));
    let drawing = run_import(&document, Some(Arc::new(options(source)))).expect("import succeeds");

    assert_eq!(
        reference(&drawing, "XREF").status,
        ExternalReferenceStatus::Loaded
    );
    assert!(
        has_diagnostic(&drawing, "import.xref_paper_skipped"),
        "{:?}",
        drawing.report.diagnostics
    );
    // Only the model-space line was merged into the xref block.
    assert_eq!(block_named(&drawing, "XREF").entities.len(), 1);
}
