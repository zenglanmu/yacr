//! Import contract tests.

use super::*;
use acadrust::entities::BoundaryPath;

fn request(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    }
}

#[test]
#[ignore = "one-off fixture generator; run with YACR_PLOT_FIXTURE_OUT"]
fn generate_plot_fixture() {
    use acadrust::io::dwg::dwg_writer::DwgWriter;
    let Some(out) = std::env::var_os("YACR_PLOT_FIXTURE_OUT") else {
        return;
    };
    let mut doc = acadrust::CadDocument::new();
    // A fresh document already has a "Layout1"; reuse it.
    let layout_handle = doc
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            acadrust::objects::ObjectType::Layout(l) if l.name == "Layout1" => Some(*handle),
            _ => None,
        })
        .expect("Layout1 exists in a new document");
    doc.add_entity_to_layout(
        acadrust::EntityType::Line(acadrust::entities::Line::from_coords(
            20.0, 20.0, 0.0, 190.0, 277.0, 0.0,
        )),
        "Layout1",
    )
    .unwrap();
    // Populate the layout's embedded plot data (A4 landscape-ish, 5mm margins).
    if let Some(acadrust::objects::ObjectType::Layout(layout)) = doc.objects.get_mut(&layout_handle)
    {
        layout.paper_size = "ISO_A4_(210.00_x_297.00_MM)".into();
        layout.paper_width = 210.0;
        layout.paper_height = 297.0;
        layout.plot_margin_left = 5.0;
        layout.plot_margin_bottom = 5.0;
        layout.plot_margin_right = 5.0;
        layout.plot_margin_top = 5.0;
        layout.plot_paper_units = 1;
        layout.plot_type = 5;
        layout.plot_rotation = 0;
        layout.plot_scale_numerator = 1.0;
        layout.plot_scale_denominator = 1.0;
    }
    // A standalone PLOTSETTINGS object too (the DXF-style path), with values
    // that differ from the embedded ones so the preferred source is observable.
    let settings_handle = doc.allocate_handle();
    let mut settings = acadrust::objects::PlotSettings::new("Layout1");
    settings.handle = settings_handle;
    settings.owner = layout_handle;
    settings.paper_size = "ISO_A3_(297.00_x_420.00_MM)".into();
    settings.paper_width = 297.0;
    settings.paper_height = 420.0;
    settings.margins = acadrust::objects::PaperMargin::new(12.0, 12.0, 12.0, 12.0);
    settings.rotation = acadrust::objects::PlotRotation::Degrees90;
    settings.paper_units = acadrust::objects::PlotPaperUnits::Millimeters;
    settings.plot_type = acadrust::objects::PlotType::Layout;
    settings.scale_numerator = 1.0;
    settings.scale_denominator = 1.0;
    doc.objects.insert(
        settings_handle,
        acadrust::objects::ObjectType::PlotSettings(settings),
    );
    let bytes = DwgWriter::write_to_vec(&doc).unwrap();
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {} bytes to {:?}", bytes.len(), out);
}

#[test]
fn garbage_input_fails_without_panicking() {
    let importer = AcadrustImporter::new();
    let result = importer.import(&request(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]), &|| {
        false
    });
    assert!(result.is_err(), "random bytes must not import as a drawing");
}

#[test]
fn oversize_input_is_rejected_before_parsing() {
    let importer = AcadrustImporter::new();
    let mut req = request(vec![0u8; 64]);
    req.limits.max_file_bytes = 8;
    assert!(matches!(
        importer.import(&req, &|| false),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn cancellation_is_honoured() {
    let importer = AcadrustImporter::new();
    let result = importer.import(&request(vec![0u8; 64]), &|| true);
    assert!(matches!(result, Err(CadError::Cancelled)));
}

/// A synthetic, writer-produced DWG with `count` model-space lines.
///
/// Uses acadrust's own writer, so the bytes are a real AC10xx DWG rather than a
/// vendored sample (no authorized fixture is required for these contract tests).
fn synthetic_dwg(count: usize) -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();
    for i in 0..count {
        let mut line = acadrust::entities::Line::new();
        line.start = acadrust::types::Vector3::new(i as f64, 0.0, 0.0);
        line.end = acadrust::types::Vector3::new(i as f64, 1.0, 0.0);
        doc.add_entity(EntityType::Line(line)).expect("add line");
    }
    acadrust::DwgWriter::write_to_vec(&doc).expect("write synthetic DWG")
}

#[test]
fn progress_reports_real_phases_in_order() {
    use std::sync::Mutex;
    let importer = AcadrustImporter::new();
    let ticks = Mutex::new(Vec::new());
    let drawing = importer
        .import_with_progress(
            &request(synthetic_dwg(3)),
            &|| false,
            &|p: ImportProgress| ticks.lock().unwrap().push(p),
        )
        .expect("synthetic drawing imports");
    assert_eq!(drawing.database.entity_count(), 3);

    let ticks = ticks.into_inner().unwrap();
    let phases: Vec<ImportPhase> = ticks.iter().map(|p| p.phase).collect();
    // Read start/end, table build, entity batches, resolve, finish — in order.
    assert_eq!(
        phases.first(),
        Some(&ImportPhase::Reading),
        "read start is the first reported boundary"
    );
    assert!(phases.contains(&ImportPhase::Parsing));
    assert!(phases.contains(&ImportPhase::Tables));
    assert!(phases.contains(&ImportPhase::Entities));
    assert!(phases.contains(&ImportPhase::Resolving));
    assert_eq!(
        phases.last(),
        Some(&ImportPhase::Finishing),
        "finish is the last boundary"
    );
    let first_index = |phase: ImportPhase| phases.iter().position(|p| *p == phase).unwrap();
    assert!(first_index(ImportPhase::Reading) < first_index(ImportPhase::Parsing));
    assert!(first_index(ImportPhase::Parsing) < first_index(ImportPhase::Tables));
    assert!(first_index(ImportPhase::Tables) < first_index(ImportPhase::Entities));
    assert!(first_index(ImportPhase::Entities) < first_index(ImportPhase::Resolving));
    assert!(first_index(ImportPhase::Resolving) < first_index(ImportPhase::Finishing));

    // Entity counts only rise, and end at the real total.
    let mut last = 0usize;
    for p in ticks.iter().filter(|p| p.phase == ImportPhase::Entities) {
        assert!(p.entities_done >= last, "counts never go backwards");
        last = p.entities_done;
        if let Some(total) = p.entities_total {
            assert!(
                p.entities_done <= total,
                "done never exceeds the real total"
            );
        }
    }
    assert_eq!(last, 3);
    // The read-start byte count is the real request length, not a guess.
    let reading = ticks.first().unwrap();
    assert!(reading.bytes.unwrap_or(0) > 0);
    assert!(
        reading.entities_total.is_none(),
        "total unknown before parse"
    );
}

#[test]
fn cancellation_during_import_returns_cancelled_without_a_database() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Cancel as soon as the first entity batch is reported. The importer polls
    // cancellation at every batch boundary and must abort with `Cancelled`.
    let seen = AtomicUsize::new(0);
    let importer = AcadrustImporter::new();
    let cancelled = || seen.load(Ordering::SeqCst) > 0;
    let result = importer.import_with_progress(
        &request(synthetic_dwg(2000)),
        &cancelled,
        &|p: ImportProgress| {
            if p.phase == ImportPhase::Entities {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    assert!(
        matches!(result, Err(CadError::Cancelled)),
        "a cancelled import must not return a partial drawing"
    );
}

#[test]
fn malformed_dwg_is_corrupt_not_cancelled() {
    let importer = AcadrustImporter::new();
    // A valid signature followed by garbage is a truncated/corrupt stream: the
    // failsafe reader recovers an empty document, which must be surfaced as
    // corrupt data rather than published as an empty success.
    let mut bytes = b"AC1032".to_vec();
    bytes.extend_from_slice(&[0xAB; 512]);
    let result = importer.import_with_progress(&request(bytes), &|| false, &NoopProgress);
    match result {
        Err(CadError::CorruptData(_)) => {}
        Err(other) => panic!("expected CorruptData, got {other:?}"),
        Ok(_) => panic!("malformed DWG must not import successfully"),
    }
}

#[test]
fn truncated_synthetic_dwg_is_corrupt() {
    let importer = AcadrustImporter::new();
    let full = synthetic_dwg(2);
    // Cut the stream before its end so `stream_completed` is false.
    let truncated = full[..full.len() / 3].to_vec();
    let result = importer.import_with_progress(&request(truncated), &|| false, &NoopProgress);
    assert!(
        matches!(result, Err(CadError::CorruptData(_))),
        "a truncated DWG must not be published as an empty success"
    );
}

#[test]
fn space_block_names_are_case_insensitive() {
    for name in ["*Model_Space", "*MODEL_SPACE", "*model_space"] {
        assert!(is_space_block_name(name), "{name}");
    }
    for name in ["*Paper_Space", "*PAPER_SPACE", "*Paper_Space0"] {
        assert!(is_space_block_name(name), "{name}");
        assert!(is_paper_space_name(name), "{name}");
    }
    assert!(!is_space_block_name("MyBlock"));
    assert!(!is_paper_space_name("*Model_Space"));
}

#[test]
fn display_support_separates_drawn_from_unrendered() {
    let line = SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    };
    assert_eq!(display_support(&line).0, SupportStatus::Verified);
    let text = SemanticGeometry::Text {
        text: "x".into(),
        position: Point3::default(),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    };
    assert_eq!(display_support(&text).0, SupportStatus::Unsupported);
    let opaque = SemanticGeometry::Opaque {
        type_key: "ACIS".into(),
        version: 1,
        payload: Vec::new(),
    };
    assert_eq!(display_support(&opaque).0, SupportStatus::Unsupported);
    let insert = SemanticGeometry::Insert {
        block: BlockId(0),
        transform: Transform3::identity(),
    };
    assert_eq!(display_support(&insert).0, SupportStatus::Unverified);
}

#[test]
fn completeness_never_reports_complete_for_unrendered_content() {
    // Text-only drawing: parsed but not drawable -> Missing, never Complete.
    let c = aggregate_completeness(
        SupportStatus::Unsupported,
        false,
        vec!["AcDbText".into()],
        vec![],
    );
    assert!(matches!(c, Completeness::Missing(_)), "{c:?}");
    // Mixed drawable + unrendered -> Partial.
    let c = aggregate_completeness(
        SupportStatus::Unsupported,
        true,
        vec!["AcDbText".into()],
        vec![],
    );
    assert!(matches!(c, Completeness::Partial(_)), "{c:?}");
    // Fully drawable -> Complete.
    assert_eq!(
        aggregate_completeness(SupportStatus::Verified, true, vec![], vec![]),
        Completeness::Complete
    );
    // Empty drawing with no fault -> Complete.
    assert_eq!(
        aggregate_completeness(SupportStatus::Verified, false, vec![], vec![]),
        Completeness::Complete
    );
    // A separate import fault is still Partial, not Complete.
    let c = aggregate_completeness(
        SupportStatus::Verified,
        false,
        vec![],
        vec!["stream".into()],
    );
    assert!(matches!(c, Completeness::Partial(_)), "{c:?}");
}

#[test]
fn dimension_placement_scales_rotates_then_translates() {
    // Scale (2,2,1), rotate +90 deg about Z, translate (1,2,3).
    let t = placement_transform(
        acadrust::types::Vector3::new(1.0, 2.0, 3.0),
        std::f64::consts::FRAC_PI_2,
        acadrust::types::Vector3::new(2.0, 2.0, 1.0),
    );
    // (1,0,0) -> scale (2,0,0) -> rotate (0,2,0) -> translate (1,4,3).
    let p = t.apply_point(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    assert!((p.x - 1.0).abs() < 1e-9, "{p:?}");
    assert!((p.y - 4.0).abs() < 1e-9, "{p:?}");
    assert!((p.z - 3.0).abs() < 1e-9, "{p:?}");
}

#[test]
fn hatch_sweeps_follow_their_winding() {
    let tau = std::f64::consts::TAU;
    let half = std::f64::consts::PI / 2.0;
    assert!((directed_sweep(half, 0.0, false) + half).abs() < 1e-9);
    assert!((directed_sweep(0.0, half, true) - half).abs() < 1e-9);
    // A full turn is kept, not collapsed to zero.
    assert!((directed_sweep(0.0, 0.0, true) - tau).abs() < 1e-9);
}

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BoundaryPath {
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut path = BoundaryPath::new();
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    path
}

fn mesh_area(geometry: &SemanticGeometry) -> Option<f64> {
    let SemanticGeometry::Compound(children) = geometry else {
        return None;
    };
    children.iter().find_map(|child| match child {
        SemanticGeometry::Mesh(m) => Some(
            m.triangles
                .iter()
                .map(|t| {
                    let a = m.vertices[t[0] as usize];
                    let b = m.vertices[t[1] as usize];
                    let c = m.vertices[t[2] as usize];
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
                    let cx = ab.y * ac.z - ab.z * ac.y;
                    let cy = ab.z * ac.x - ab.x * ac.z;
                    let cz = ab.x * ac.y - ab.y * ac.x;
                    (cx * cx + cy * cy + cz * cz).sqrt() / 2.0
                })
                .sum(),
        ),
        _ => None,
    })
}

#[test]
fn solid_hatch_with_a_hole_fills_the_solid_band_only() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(3.0, 3.0, 7.0, 7.0),
    ];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let area = mesh_area(&geometry).expect("solid fill mesh");
    assert!((area - 84.0).abs() < 1e-6, "hole area not excluded: {area}");
}

fn mesh_of(geometry: &SemanticGeometry) -> Option<&Mesh> {
    let SemanticGeometry::Compound(children) = geometry else {
        return None;
    };
    children.iter().find_map(|child| match child {
        SemanticGeometry::Mesh(m) => Some(m),
        _ => None,
    })
}

fn gradient_hatch(name: &str, stops: &[(f64, (u8, u8, u8))]) -> acadrust::entities::Hatch {
    let mut hatch = acadrust::entities::Hatch::new();
    // A real gradient HATCH is stored solid with gradient metadata, so the
    // importer must prefer the gradient over the solid flag.
    hatch.is_solid = true;
    hatch.paths = vec![rect_path(0.0, 0.0, 10.0, 10.0)];
    let g = &mut hatch.gradient_color;
    g.enabled = true;
    g.name = name.to_string();
    g.angle = 0.0;
    g.is_single_color = false;
    g.color_tint = 0.0;
    for (value, (r, gg, b)) in stops {
        g.colors.push(acadrust::entities::GradientColorEntry {
            value: *value,
            color: acadrust::Color::from_rgb(*r, *gg, *b),
        });
    }
    hatch
}

#[test]
fn linear_gradient_hatch_is_complete_and_graded() {
    let hatch = gradient_hatch("LINEAR", &[(0.0, (255, 0, 0)), (1.0, (0, 0, 255))]);
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let mesh = mesh_of(&geometry).expect("gradient fill mesh");
    assert_eq!(mesh.colors.len(), mesh.vertices.len());
    let min_x = mesh
        .vertices
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = mesh
        .vertices
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let left = mesh
        .vertices
        .iter()
        .position(|p| (p.x - min_x).abs() < 1e-9)
        .expect("left vertex");
    let right = mesh
        .vertices
        .iter()
        .position(|p| (p.x - max_x).abs() < 1e-9)
        .expect("right vertex");
    assert_eq!(mesh.colors[left], [255, 0, 0], "left stop");
    assert_eq!(mesh.colors[right], [0, 0, 255], "right stop");
}

#[test]
fn spherical_gradient_hatch_is_complete() {
    // A triangle boundary has vertices at more than one radius from the bounds
    // midpoint, so the radial ramp is visible (a rectangle would collapse to a
    // constant, a documented tessellation gap).
    let mut hatch = gradient_hatch("SPHERICAL", &[(0.0, (0, 255, 0)), (1.0, (0, 0, 0))]);
    let corners = [(0.0f64, 0.0f64), (10.0, 0.0), (5.0, 10.0)];
    let mut path = BoundaryPath::new();
    for i in 0..3 {
        let a = corners[i];
        let b = corners[(i + 1) % 3];
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    hatch.paths = vec![path];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let mesh = mesh_of(&geometry).expect("gradient fill mesh");
    assert_eq!(mesh.colors.len(), mesh.vertices.len());
    assert!(mesh.colors.iter().any(|c| c[1] > 0), "green ramp expected");
}

#[test]
fn curved_gradient_kind_is_partial_with_a_stable_reason_code() {
    let hatch = gradient_hatch("HEMISPHERICAL", &[(0.0, (255, 0, 0)), (1.0, (0, 0, 255))]);
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons
                    .iter()
                    .any(|r| r.contains("gradient_kind_not_supported")),
                "{reasons:?}"
            );
            assert!(reasons
                .iter()
                .any(|r| r.contains("gradient hatch not rendered")));
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    assert!(mesh_of(&geometry).is_none(), "no gradient mesh expected");
}

#[test]
fn gradient_without_stops_is_partial_not_solid() {
    let hatch = gradient_hatch("LINEAR", &[]);
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons
                    .iter()
                    .any(|r| r.contains("gradient_definition_unusable")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    assert!(mesh_of(&geometry).is_none(), "no mesh expected");
}

#[test]
fn single_color_gradient_hatch_is_complete_and_graded() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![rect_path(0.0, 0.0, 10.0, 10.0)];
    let g = &mut hatch.gradient_color;
    g.enabled = true;
    g.name = "LINEAR".to_string();
    g.angle = 0.0;
    g.is_single_color = true;
    g.color_tint = 1.0;
    g.colors.push(acadrust::entities::GradientColorEntry {
        value: 0.0,
        color: acadrust::Color::from_rgb(255, 0, 0),
    });
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let mesh = mesh_of(&geometry).expect("gradient fill mesh");
    assert!(mesh.colors.iter().any(|c| c[0] == 255 && c[1] > 200));
}

#[test]
fn solid_hatch_without_gradient_still_has_no_vertex_colors() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![rect_path(0.0, 0.0, 4.0, 4.0)];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete);
    let mesh = mesh_of(&geometry).expect("solid mesh");
    assert!(
        mesh.colors.is_empty(),
        "solid fill has no per-vertex colours"
    );
}

#[test]
fn over_budget_multi_ring_hatch_stays_partial_boundary_only() {
    // A zig-zag star defeats Douglas-Peucker, so the loop stays over
    // MAX_FILL_POINTS and the fill must be refused, never approximated.
    let points = 2200usize;
    let mut path = BoundaryPath::new();
    for i in 0..points {
        let t = i as f64 / points as f64 * std::f64::consts::TAU;
        let r = if i % 2 == 0 { 10.0 } else { 1.0 };
        let a = (r * t.cos(), r * t.sin());
        let t2 = (i + 1) as f64 / points as f64 * std::f64::consts::TAU;
        let r2 = if (i + 1) % 2 == 0 { 10.0 } else { 1.0 };
        let b = (r2 * t2.cos(), r2 * t2.sin());
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![path, rect_path(0.0, 0.0, 20.0, 20.0)];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("budget")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    // The boundary loops are still present; no fill mesh was fabricated.
    assert!(mesh_area(&geometry).is_none(), "{geometry:?}");
}

#[test]
fn degenerate_hatch_normal_reports_partial_boundary_only() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.normal = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    hatch.paths = vec![rect_path(0.0, 0.0, 4.0, 4.0)];
    let (_geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("normal")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

// ---- B23/B31: OCS normalisation of 2D polylines (importer side) ----

fn x_axis() -> Point3 {
    Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    }
}

#[test]
fn world_z_extrusion_is_left_untouched() {
    // The common case must stay exactly `(x, y, elevation)`.
    let pts = polyline_ocs_points(world_z(), 7.0, [(1.0, 2.0), (3.0, 4.0)]);
    assert_eq!(
        pts,
        vec![
            Point3 {
                x: 1.0,
                y: 2.0,
                z: 7.0
            },
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 7.0
            },
        ]
    );
}

#[test]
fn non_z_extrusion_is_transformed_to_wcs_not_treated_as_flat() {
    // Extrusion +X: the AutoCAD arbitrary axis gives ax=+Y, ay=+Z, so an
    // OCS point (x, y) at elevation e maps to (e, x, y). The old code
    // returned (x, y, e) and treated the tilted entity as flat.
    let p = ocs_to_wcs(x_axis(), 5.0, 2.0, 3.0);
    assert!((p.x - 5.0).abs() < 1e-9, "{p:?}");
    assert!((p.y - 2.0).abs() < 1e-9, "{p:?}");
    assert!((p.z - 3.0).abs() < 1e-9, "{p:?}");
    // The mapped point must lie on the extrusion plane through elevation.
    assert!(!is_world_z(x_axis()));
}

#[test]
fn tilted_lwpolyline_vertices_carry_the_ocs_plane() {
    // A real acadrust entity, no DWG needed: a two-vertex LWPOLYLINE with
    // an +X extrusion and elevation 4.
    let mut pl = acadrust::entities::LwPolyline::from_points(vec![
        acadrust::types::Vector2::new(1.0, 0.0),
        acadrust::types::Vector2::new(0.0, 2.0),
    ]);
    pl.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    pl.elevation = 4.0;

    let points = polyline_ocs_points(
        p3(pl.normal),
        pl.elevation,
        pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
    );
    // (1,0) -> (4,1,0); (0,2) -> (4,0,2).
    assert_eq!(
        points,
        vec![
            Point3 {
                x: 4.0,
                y: 1.0,
                z: 0.0
            },
            Point3 {
                x: 4.0,
                y: 0.0,
                z: 2.0
            },
        ]
    );
    // Both vertices share the plane x = elevation; not flat on XY.
    assert!(points.iter().all(|p| (p.x - 4.0).abs() < 1e-9));
    assert!(points.iter().any(|p| p.z.abs() > 1e-9));
}

#[test]
fn tilted_bulge_completeness_tracks_plane_representability() {
    // A tilted polyline with >=3 vertices fixes its own plane, so a bulge
    // is exact and the import is Complete. A two-vertex tilted bulge has no
    // unique plane and must not claim Complete.
    assert_eq!(
        polyline_completeness(x_axis(), 4, &[0.0, 0.0, 0.0, 0.0]),
        Completeness::Complete
    );
    assert_eq!(
        polyline_completeness(x_axis(), 4, &[0.5, 0.0, 0.0, 0.0]),
        Completeness::Complete
    );
    assert!(matches!(
        polyline_completeness(x_axis(), 2, &[0.5, 0.0]),
        Completeness::Partial(_)
    ));
    // A world-Z bulge polyline is always fully supported.
    assert_eq!(
        polyline_completeness(world_z(), 2, &[0.5, 0.0]),
        Completeness::Complete
    );
}

#[test]
fn tilted_ellipse_keeps_its_normal_and_is_complete() {
    // A +X extrusion is now carried exactly (the domain Ellipse has a
    // normal), so the ellipse is no longer flattened onto world XY.
    let mut e = acadrust::entities::Ellipse::new();
    e.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    e.center = acadrust::types::Vector3::new(2.0, 3.0, 4.0);
    e.major_axis = acadrust::types::Vector3::new(0.0, 5.0, 0.0);
    let (geom, completeness) = ellipse_semantics(&e);
    match geom {
        SemanticGeometry::Ellipse {
            normal,
            major_axis,
            ratio,
            ..
        } => {
            assert!((normal.x - 1.0).abs() < 1e-12, "normal {normal:?}");
            assert!(
                cad_geometry::length(major_axis) >= 4.0,
                "major axis lost: {major_axis:?}"
            );
            assert!(ratio > 0.0);
        }
        other => panic!("expected ellipse, got {other:?}"),
    }
    assert_eq!(completeness, Completeness::Complete);

    // A degenerate extrusion defaults to world Z rather than collapsing.
    e.normal = acadrust::types::Vector3::ZERO;
    match ellipse_semantics(&e).0 {
        SemanticGeometry::Ellipse { normal, .. } => {
            assert!((normal.z - 1.0).abs() < 1e-12, "normal {normal:?}");
        }
        other => panic!("expected ellipse, got {other:?}"),
    }
}

#[test]
fn tilted_circle_centre_is_mapped_from_ocs_to_wcs() {
    // A CIRCLE stores its centre in OCS. With a +X extrusion the arbitrary
    // axis frame maps OCS (x, y) at elevation z to WCS (z, x, y); reading
    // the centre verbatim would put the circle in the wrong plane.
    let mut c = acadrust::entities::Circle::new();
    c.center = acadrust::types::Vector3::new(2.0, 3.0, 5.0);
    c.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    let wcs = c.center_wcs();
    let expected = ocs_to_wcs(
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        5.0,
        2.0,
        3.0,
    );
    assert!((wcs.x - expected.x).abs() < 1e-9);
    assert!((wcs.y - expected.y).abs() < 1e-9);
    assert!((wcs.z - expected.z).abs() < 1e-9);
}

// ---- B22: paper-space viewport → four corners + full transform ----

/// Build a top/plan VIEWPORT entity with the given paper rectangle, scale,
/// and model view target.
fn top_viewport(
    center: (f64, f64),
    w: f64,
    h: f64,
    view_height: f64,
    view_target: (f64, f64),
) -> acadrust::entities::Viewport {
    let mut v = acadrust::entities::Viewport::new();
    v.id = 2;
    v.center = acadrust::types::Vector3::new(center.0, center.1, 0.0);
    v.width = w;
    v.height = h;
    v.view_height = view_height;
    v.view_direction = acadrust::types::Vector3::UNIT_Z;
    v.view_target = acadrust::types::Vector3::new(view_target.0, view_target.1, 0.0);
    v
}

#[test]
fn one_to_one_hundred_viewport_has_four_corners_and_the_real_transform() {
    // 1:100: a 10000×5000 model region fits the 100×50 paper window, view
    // centre (10, 20).
    let vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    let pv = paper_viewport(&vp).expect("id 2 is a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(
        pv.clip,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 100.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 100.0,
                y: 50.0,
                z: 0.0
            },
            Point3 {
                x: 0.0,
                y: 50.0,
                z: 0.0
            },
        ]
    );
    // The stored paper→model transform maps the paper centre to the view
    // target; 100 model units per paper unit.
    let m = &pv.model_to_paper.matrix;
    assert!((m[0][0] - 100.0).abs() < 1e-9, "{m:?}");
    assert!((m[1][1] - 100.0).abs() < 1e-9, "{m:?}");
    let model_centre = pv.model_to_paper.apply_point(Point3 {
        x: 50.0,
        y: 25.0,
        z: 0.0,
    });
    assert!((model_centre.x - 10.0).abs() < 1e-9, "{model_centre:?}");
    assert!((model_centre.y - 20.0).abs() < 1e-9, "{model_centre:?}");
    // One paper unit right of centre is 100 model units.
    let right = pv.model_to_paper.apply_point(Point3 {
        x: 51.0,
        y: 25.0,
        z: 0.0,
    });
    assert!((right.x - 110.0).abs() < 1e-9, "{right:?}");
}

#[test]
fn sheet_viewport_and_off_viewport_are_not_content_viewports() {
    let mut sheet = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
    sheet.id = 1;
    assert!(paper_viewport(&sheet).is_none(), "id 1 is the sheet");
    let mut off = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
    off.status.is_on = false;
    assert!(
        paper_viewport(&off).is_none(),
        "an off viewport draws nothing"
    );
}

#[test]
fn rotated_viewport_is_partial_with_a_twist_reason() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.twist_angle = std::f64::consts::FRAC_PI_4;
    let pv = paper_viewport(&vp).expect("still a content viewport");
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("twist")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    // The four-corner rectangle is still present (the representation layer
    // will refuse it with `viewport.twisted_transform`, never square it off).
    assert_eq!(pv.clip.len(), 4);
}

#[test]
fn non_perpendicular_view_is_partial_with_an_off_plane_reason() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.view_direction = acadrust::types::Vector3::new(1.0, 0.0, 1.0);
    let pv = paper_viewport(&vp).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r.contains("perpendicular")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn complex_clip_viewport_is_partial() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = acadrust::types::Handle::new(0x1A);
    let pv = paper_viewport(&vp).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

// ---- B31: INSERT base point, OCS normal and array semantics ----

#[test]
fn insert_subtracts_the_block_base_point() {
    // A block whose base point is (1, 1) and an INSERT at (10, 10): the
    // block point (1, 1) must land on (10, 10), not (11, 11).
    let mut i =
        acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::new(10.0, 10.0, 0.0));
    i.rotation = 0.0;
    let t = insert_array_transform(
        &i,
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        0.0,
        0.0,
    );
    let placed = t.apply_point(Point3 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
    });
    assert!((placed.x - 10.0).abs() < 1e-9, "{placed:?}");
    assert!((placed.y - 10.0).abs() < 1e-9, "{placed:?}");
    // The origin with base (1,1) lands one block unit left/below the insert.
    let origin = t.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    assert!((origin.x - 9.0).abs() < 1e-9, "{origin:?}");
    assert!((origin.y - 9.0).abs() < 1e-9, "{origin:?}");
}

#[test]
fn insert_positive_rotation_turns_counter_clockwise() {
    let mut i = acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::ZERO);
    i.rotation = std::f64::consts::FRAC_PI_2;
    // Base point (1,0). A block point (2,0) is (1,0) after base subtraction;
    // +90° CCW turns it to (0,1).
    let t = insert_array_transform(
        &i,
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        0.0,
        0.0,
    );
    let placed = t.apply_point(Point3 {
        x: 2.0,
        y: 0.0,
        z: 0.0,
    });
    assert!(placed.x.abs() < 1e-9, "{placed:?}");
    assert!((placed.y - 1.0).abs() < 1e-9, "{placed:?}");
    // The base point itself lands on the insert point (the origin).
    let base = t.apply_point(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    assert!(base.x.abs() < 1e-9 && base.y.abs() < 1e-9, "{base:?}");
}

#[test]
fn insert_ocs_normal_lifts_the_block_off_the_xy_plane() {
    // A +X extrusion maps the OCS X/Y axes into world Y/Z, so a block
    // point on its local X lands off the world XY plane (audit B31).
    let mut i = acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::ZERO);
    i.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    let t = insert_array_transform(&i, Point3::default(), 0.0, 0.0);
    let w = t.apply_point(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    // arbitrary_axis(+X) = (ax=+Y, ay=+Z); OCS (1,0) -> world (0,1).
    assert!(w.x.abs() < 1e-9, "{w:?}");
    assert!((w.y - 1.0).abs() < 1e-9, "{w:?}");
    assert!(w.z.abs() < 1e-9, "{w:?}");
}

#[test]
fn insert_array_offsets_are_pre_scale_and_row_major() {
    let mut i = acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::ZERO);
    // Non-uniform scale: the 10-unit column spacing must not be scaled by
    // the 2× x-scale.
    i.set_x_scale(2.0);
    i.set_y_scale(3.0);
    i.column_count = 2;
    i.row_count = 2;
    i.column_spacing = 10.0;
    i.row_spacing = 20.0;

    let cell = |col: usize, row: usize| {
        insert_array_transform(
            &i,
            Point3::default(),
            col as f64 * i.column_spacing,
            row as f64 * i.row_spacing,
        )
        .apply_point(Point3::default())
    };
    // Row-major: cells are (col 0,row 0), (col 1,row 0), ...
    assert_eq!(cell(0, 0).x, 0.0);
    assert_eq!(cell(1, 0).x, 10.0);
    assert_eq!(cell(0, 1).x, 0.0);
    assert_eq!(cell(0, 1).y, 20.0);
}

#[test]
fn array_insert_is_a_compound_of_cell_instances() {
    // referenced_blocks must see every cell so status nesting resolves
    // through MINSERTs; insert_semantics builds one Instance per cell.
    let geometry = SemanticGeometry::Compound(vec![
        SemanticGeometry::Insert {
            block: BlockId(7),
            transform: Transform3::identity(),
        },
        SemanticGeometry::Insert {
            block: BlockId(7),
            transform: Transform3::identity(),
        },
    ]);
    assert_eq!(referenced_blocks(&geometry), vec![BlockId(7), BlockId(7)]);
    assert!(referenced_blocks(&SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3::default(),
    })
    .is_empty());
}

#[test]
fn missing_block_is_reported_missing_not_an_empty_success() {
    // A bare builder (no block table) resolves the insert block to the
    // sentinel id; the record must be `Missing`, never a silent success.
    let bytes = vec![0u8; 0];
    let req = request(bytes);
    let acad = acadrust::CadDocument::new();
    let stats = ReadStats::default();
    let builder = ImporterBuilder::new(&req, &acad, stats, compute_identity(&[]));
    let mut i = acadrust::entities::Insert::new("NOPE", acadrust::types::Vector3::ZERO);
    i.column_count = 1;
    i.row_count = 1;
    let (geometry, completeness) = builder.insert_semantics(&i);
    assert!(matches!(geometry, SemanticGeometry::Insert { .. }));
    match completeness {
        Completeness::Missing(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("NOPE")), "{reasons:?}");
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

// ---- B31: SOLID/TRACE boundary order and OCS lift ----

#[test]
fn solid_corners_use_the_visible_boundary_order() {
    // Stored corners 1,2,3,4 with the visible quad 1,2,4,3. The extruded
    // triangles must follow the boundary, not the stored order.
    let mut s = acadrust::entities::Solid::new(
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(2.0, 0.0, 0.0),
        acadrust::types::Vector3::new(0.0, 2.0, 0.0),
        acadrust::types::Vector3::new(2.0, 2.0, 0.0),
    );
    s.normal = acadrust::types::Vector3::UNIT_Z;
    let (geometry, completeness) = solid_mesh_semantics(&s);
    assert_eq!(completeness, Completeness::Complete);
    let SemanticGeometry::Mesh(mesh) = geometry else {
        panic!("expected a mesh");
    };
    // Boundary order: (0,0), (2,0), (2,2), (0,2). Triangles (0,1,2) and
    // (0,2,3) must be two triangles of a unit square (area 4 total).
    let area: f64 = mesh
        .triangles
        .iter()
        .map(|t| {
            let a = mesh.vertices[t[0] as usize];
            let b = mesh.vertices[t[1] as usize];
            let c = mesh.vertices[t[2] as usize];
            let ab = Point3 {
                x: b.x - a.x,
                y: b.y - a.y,
                z: 0.0,
            };
            let ac = Point3 {
                x: c.x - a.x,
                y: c.y - a.y,
                z: 0.0,
            };
            (ab.x * ac.y - ab.y * ac.x).abs() / 2.0
        })
        .sum();
    assert!((area - 4.0).abs() < 1e-9, "crossed quad area {area}");
}

#[test]
fn solid_with_a_non_z_extrusion_is_lifted_to_wcs() {
    let mut s = acadrust::entities::Solid::new(
        acadrust::types::Vector3::new(1.0, 0.0, 0.0),
        acadrust::types::Vector3::new(0.0, 1.0, 0.0),
        acadrust::types::Vector3::new(0.0, 0.0, 1.0),
        acadrust::types::Vector3::new(1.0, 1.0, 1.0),
    );
    s.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    let (geometry, completeness) = solid_mesh_semantics(&s);
    assert_eq!(completeness, Completeness::Complete);
    let SemanticGeometry::Mesh(mesh) = geometry else {
        panic!("expected a mesh");
    };
    // With +X extrusion the arbitrary-axis frame maps OCS (x, y, z) to
    // world (z, x, y). The old flat treatment would have left the first
    // corner at (1,0,0); the lift must move it to (0,1,0).
    assert!(mesh
        .vertices
        .iter()
        .any(|p| { (p.x).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9 && p.z.abs() < 1e-9 }));
    // The fourth OCS corner (1,1,1) -> world (1,1,1).
    assert!(mesh.vertices.iter().any(|p| {
        (p.x - 1.0).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9 && (p.z - 1.0).abs() < 1e-9
    }));
}

#[test]
fn solid_thickness_is_partial_flat_face_only() {
    let mut s = acadrust::entities::Solid::new(
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(1.0, 0.0, 0.0),
        acadrust::types::Vector3::new(0.0, 1.0, 0.0),
        acadrust::types::Vector3::new(1.0, 1.0, 0.0),
    );
    s.thickness = 5.0;
    let (_geometry, completeness) = solid_mesh_semantics(&s);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r.contains("thickness")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

// ---- B23/B31: tilted OCS polyline length ----

#[test]
fn tilted_ocs_polyline_preserves_its_segment_length() {
    // A 3-4-5 triangle drawn flat on the OCS XY plane at an +X extrusion.
    // Lifting it to WCS must preserve each segment's length exactly, while
    // the old flat treatment would have collapsed it onto the XY plane.
    let mut pl = acadrust::entities::LwPolyline::from_points(vec![
        acadrust::types::Vector2::new(0.0, 0.0),
        acadrust::types::Vector2::new(3.0, 0.0),
        acadrust::types::Vector2::new(3.0, 4.0),
    ]);
    pl.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
    pl.elevation = 7.0;
    let points = polyline_ocs_points(
        p3(pl.normal),
        pl.elevation,
        pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
    );
    let seg = |a: Point3, b: Point3| {
        let d = Point3 {
            x: b.x - a.x,
            y: b.y - a.y,
            z: b.z - a.z,
        };
        (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
    };
    assert!((seg(points[0], points[1]) - 3.0).abs() < 1e-9, "{points:?}");
    assert!((seg(points[1], points[2]) - 4.0).abs() < 1e-9, "{points:?}");
    // Both vertices lie on the x = elevation plane.
    assert!(points.iter().all(|p| (p.x - 7.0).abs() < 1e-9));
}

// ---- B23: source spline knots and weights survive the importer ----

#[test]
fn source_spline_knots_and_weights_survive_import() {
    let mut s = acadrust::entities::Spline::new();
    s.degree = 2;
    // Explicit non-uniform clamped knots, not the uniform ones acadrust
    // would fabricate.
    s.knots = vec![0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0];
    s.control_points = vec![
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(1.0, 2.0, 0.0),
        acadrust::types::Vector3::new(2.0, 0.0, 0.0),
        acadrust::types::Vector3::new(3.0, 2.0, 0.0),
        acadrust::types::Vector3::new(4.0, 0.0, 0.0),
    ];
    s.weights = vec![1.0, 3.0, 1.0, 3.0, 1.0];

    let (geometry, completeness) = spline_semantics(&s);
    match geometry {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            assert_eq!(degree, 2);
            assert_eq!(knots, s.knots, "source knots must not be uniformised");
            assert_eq!(weights, s.weights, "source weights must survive");
            assert_eq!(control_points.len(), 5);
        }
        other => panic!("expected spline, got {other:?}"),
    }
    // Rational splines are evaluated from the source knots and weights, so
    // the record is complete rather than downgraded.
    assert_eq!(completeness, Completeness::Complete);

    // A mismatched weight vector is the honest Partial case.
    s.weights = vec![1.0, 3.0];
    assert!(matches!(spline_semantics(&s).1, Completeness::Partial(_)));
}

// ---- F14/B21: transparency resolution + proxy fragment preservation ----

#[test]
fn transparency_resolution_prefers_byobject_then_layer_then_byblock() {
    // acadrust packs transparency as a byte: 0 opaque, 255 transparent.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::new(0), 0.5),
        EntityTransparency::Explicit(1.0)
    );
    // ByObject (Explicit) overrides the layer value.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::new(128), 0.5),
        EntityTransparency::Explicit(1.0 - 128.0 / 255.0)
    );
    // ByLayer uses the pre-resolved layer opacity.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::BY_LAYER, 0.25),
        EntityTransparency::Explicit(0.25)
    );
    // ByBlock stays symbolic so INSERT expansion can supply the value.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::BY_BLOCK, 0.25),
        EntityTransparency::ByBlock
    );
}

#[test]
fn layer_opacity_maps_dwg_bytes_to_opacity() {
    assert_eq!(layer_opacity(acadrust::Transparency::OPAQUE), 1.0);
    assert_eq!(layer_opacity(acadrust::Transparency::TRANSPARENT), 0.0);
    // A degenerate ByLayer/ByBlock on a layer is opaque, not fabricated.
    assert_eq!(layer_opacity(acadrust::Transparency::BY_LAYER), 1.0);
}

// ---- §3.2/§7.1: colour and lineweight resolution ----

#[test]
fn color_resolution_prefers_byobject_then_layer_then_byblock() {
    // ByObject true colour wins over the layer.
    assert_eq!(
        resolve_entity_color(acadrust::Color::from_rgb(10, 20, 30), Some([1, 2, 3])),
        EntityColor::Explicit([10, 20, 30])
    );
    // An ACI index resolves through acadrust's canonical table, not a guess.
    assert_eq!(
        resolve_entity_color(acadrust::Color::Index(1), Some([1, 2, 3])),
        EntityColor::Explicit([255, 0, 0])
    );
    // ByLayer uses the layer's pre-resolved colour.
    assert_eq!(
        resolve_entity_color(acadrust::Color::ByLayer, Some([1, 2, 3])),
        EntityColor::Explicit([1, 2, 3])
    );
    // A materialised `None` keeps the layer's colour rather than black.
    assert_eq!(
        resolve_entity_color(acadrust::Color::None, Some([4, 5, 6])),
        EntityColor::Explicit([4, 5, 6])
    );
    // ByBlock stays symbolic so INSERT expansion can supply the value.
    assert_eq!(
        resolve_entity_color(acadrust::Color::ByBlock, Some([4, 5, 6])),
        EntityColor::ByBlock
    );
    // ByLayer with no reachable layer stays unresolved, not fabricated.
    assert_eq!(
        resolve_entity_color(acadrust::Color::ByLayer, None),
        EntityColor::ByLayer
    );
}

#[test]
fn layer_rgb_resolves_index_and_rgb_and_falls_back_to_white() {
    assert_eq!(layer_rgb(acadrust::Color::from_rgb(9, 8, 7)), [9, 8, 7]);
    assert_eq!(layer_rgb(acadrust::Color::Index(5)), [0, 0, 255]);
    // A degenerate symbolic layer colour falls back to white.
    assert_eq!(layer_rgb(acadrust::Color::ByLayer), [255, 255, 255]);
}

#[test]
fn lineweight_resolution_prefers_byobject_then_layer_then_byblock() {
    // A concrete weight is 1/100 mm; 35 -> 0.35 mm.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::Value(35), Some(0.5)),
        EntityLineWeight::Explicit(0.35)
    );
    // ByLayer uses the layer's pre-resolved weight.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::ByLayer, Some(0.5)),
        EntityLineWeight::Explicit(0.5)
    );
    // acadrust's Default keeps its explicit meaning.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::Default, Some(0.5)),
        EntityLineWeight::Default
    );
    // ByBlock stays symbolic.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::ByBlock, Some(0.5)),
        EntityLineWeight::ByBlock
    );
    // ByLayer with no reachable layer stays unresolved.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::ByLayer, None),
        EntityLineWeight::ByLayer
    );
}

#[test]
fn lineweight_mm_only_reports_concrete_values() {
    assert_eq!(lineweight_mm(acadrust::LineWeight::Value(100)), Some(1.0));
    assert_eq!(lineweight_mm(acadrust::LineWeight::W0_25), Some(0.25));
    assert_eq!(lineweight_mm(acadrust::LineWeight::ByLayer), None);
    assert_eq!(lineweight_mm(acadrust::LineWeight::ByBlock), None);
    assert_eq!(lineweight_mm(acadrust::LineWeight::Default), None);
}

// ---- §3.2/§7.1: linetype resolution ----

fn dashed_pattern() -> LinetypePattern {
    LinetypePattern::from_elements([0.5, -0.25])
}

#[test]
fn linetype_resolution_prefers_explicit_then_layer_then_byblock() {
    let dashed = dashed_pattern();
    // An explicit named linetype wins over the layer.
    assert_eq!(
        resolve_entity_linetype("Dashed", 2.0, Some(dashed.clone()), Some(dashed_pattern())),
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: dashed.clone(),
            scale: 2.0,
        }
    );
    // ByLayer uses the layer's pre-resolved pattern (and the entity's scale).
    assert_eq!(
        resolve_entity_linetype("ByLayer", 1.5, None, Some(dashed.clone())),
        EntityLineType::Explicit {
            name: "ByLayer".into(),
            pattern: dashed.clone(),
            scale: 1.5,
        }
    );
    // An empty name is also ByLayer.
    assert_eq!(
        resolve_entity_linetype("", 1.0, None, Some(dashed.clone())),
        EntityLineType::Explicit {
            name: "ByLayer".into(),
            pattern: dashed,
            scale: 1.0,
        }
    );
    // ByBlock stays symbolic so INSERT expansion can substitute the value.
    assert_eq!(
        resolve_entity_linetype("ByBlock", 1.0, None, Some(dashed_pattern())),
        EntityLineType::ByBlock
    );
    // ByLayer with no reachable pattern is symbolic, not fabricated.
    assert_eq!(
        resolve_entity_linetype("ByLayer", 1.0, None, None),
        EntityLineType::ByLayer
    );
}

#[test]
fn unknown_named_linetype_is_an_explicit_continuous_fallback() {
    // The caller passes `None` for an unknown name; the entity gets an explicit
    // continuous pattern carrying the source name, never invented dashes.
    let resolved = resolve_entity_linetype("NoSuchLine", 1.0, None, None);
    match resolved {
        EntityLineType::Explicit {
            name,
            pattern,
            scale,
        } => {
            assert_eq!(name, "NoSuchLine");
            assert!(pattern.is_continuous());
            assert_eq!(scale, 1.0);
        }
        other => panic!("expected explicit continuous fallback, got {other:?}"),
    }
}

#[test]
fn standard_continuous_linetype_is_explicit_and_solid() {
    let resolved = resolve_entity_linetype("Continuous", 1.0, None, None);
    match resolved {
        EntityLineType::Explicit { pattern, .. } => assert!(pattern.is_continuous()),
        other => panic!("expected explicit continuous, got {other:?}"),
    }
}

#[test]
fn non_finite_linetype_scale_falls_back_to_one() {
    let resolved = resolve_entity_linetype("Dashed", f64::NAN, Some(dashed_pattern()), None);
    match resolved {
        EntityLineType::Explicit { scale, .. } => assert_eq!(scale, 1.0),
        other => panic!("expected explicit, got {other:?}"),
    }
    let resolved = resolve_entity_linetype("Dashed", -2.0, Some(dashed_pattern()), None);
    match resolved {
        EntityLineType::Explicit { scale, .. } => assert_eq!(scale, 1.0),
        other => panic!("expected explicit, got {other:?}"),
    }
}

#[test]
fn linetype_pattern_keeps_complex_flag_and_skips_non_finite() {
    let mut lt = acadrust::LineType::new("Fenceline");
    lt.add_element(acadrust::tables::LineTypeElement::dash(1.0));
    lt.add_element(acadrust::tables::LineTypeElement::space(0.5));
    let (pattern, complex) = linetype_pattern(&lt);
    assert!(!complex);
    assert_eq!(pattern.elements, vec![1.0, -0.5]);
    assert!((pattern.cycle - 1.5).abs() < 1e-12);

    let mut complex_lt = acadrust::LineType::new("Gasline");
    let mut elem = acadrust::tables::LineTypeElement::dash(1.0);
    elem.complex = Some(acadrust::tables::LineTypeComplexData::default());
    complex_lt.add_element(elem);
    complex_lt.add_element(acadrust::tables::LineTypeElement::space(0.5));
    let (pattern, complex) = linetype_pattern(&complex_lt);
    assert!(complex, "a shape/text element must be flagged");
    assert_eq!(pattern.elements, vec![1.0, -0.5]);
}

#[test]
fn pattern_builder_drops_non_finite_and_keeps_valid_elements() {
    let pattern = LinetypePattern::from_elements([1.0, f64::NAN, -0.5, f64::INFINITY]);
    assert_eq!(pattern.elements, vec![1.0, -0.5]);
    assert!((pattern.cycle - 1.5).abs() < 1e-12);
    // A pattern with only non-finite elements degrades to continuous.
    assert!(LinetypePattern::from_elements([f64::NAN]).is_continuous());
    assert!(LinetypePattern::from_elements([0.0, 0.0]).is_continuous());
}

#[test]
fn all_proxy_fragments_survive_as_a_compound() {
    let fragment = |id: u128| SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3 {
            x: id as f64,
            y: 0.0,
            z: 0.0,
        },
    };
    match proxy_geometry_compound(vec![fragment(1), fragment(2), fragment(3)]) {
        SemanticGeometry::Compound(children) => {
            assert_eq!(children.len(), 3, "every proxy fragment must survive");
        }
        other => panic!("expected a compound, got {other:?}"),
    }
    // A single fragment stays itself: no needless wrapper.
    assert!(matches!(
        proxy_geometry_compound(vec![fragment(1)]),
        SemanticGeometry::Line { .. }
    ));
}

#[test]
fn proxy_cache_is_only_used_for_entities_without_semantic_geometry() {
    let line = EntityType::Line(acadrust::entities::Line::new());
    assert!(
        !proxy_geometry_allowed(&line),
        "a LINE is drawn from semantics; its cache must not double-draw"
    );
    let unknown = EntityType::Unknown(acadrust::entities::UnknownEntity::new("ACAD_PROXY_ENTITY"));
    assert!(proxy_geometry_allowed(&unknown));
    let vendor = EntityType::Unknown(acadrust::entities::UnknownEntity::new("TCH_WALL"));
    assert!(proxy_geometry_allowed(&vendor));
}

// ---- ACIS neutral lift and kernel tessellation (F15 reachable subset) ----

use cad_kernel_adapter::{
    BrepSurface, BrepTessellator, GeometryHandle, SolidExchange, SolidTessellator,
    TessellationBudget, TessellationOutcome, TessellationRequest, TessellationResult,
    TessellationTolerance,
};

fn tess_brep(exchange: SolidExchange) -> TessellationResult {
    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(1)),
        exchange,
        tolerance: TessellationTolerance::default(),
        budget: TessellationBudget::default(),
        stamp: TaskStamp::new(DocumentId(1), 0),
    };
    BrepTessellator.tessellate(&request, &|| false).unwrap()
}

#[test]
fn acis_box_lifts_to_six_planar_faces_and_tessellates_closed() {
    use acadrust::entities::acis::primitives::build_box;
    let doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 6, "a box has six faces");
    assert!(brep
        .shells
        .iter()
        .flat_map(|s| &s.faces)
        .all(|f| matches!(f.surface, BrepSurface::Plane { .. })));

    let result = tess_brep(SolidExchange::Brep(brep));
    match result.outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert_eq!(geometry.triangle_count(), 12);
            let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
            assert!((area - 24.0).abs() < 1e-9, "box area {area}");
        }
        other => panic!("box must tessellate as Success, got {other:?}"),
    }
}

#[test]
fn solid3d_entity_round_trips_through_sat_text() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::Solid3D;
    let sat = build_box([1.0, 2.0, 3.0], 2.0, 4.0, 6.0).to_sat_string();
    let solid = Solid3D::from_sat(&sat);
    let exchange = solid_exchange_from_entity(&EntityType::Solid3D(solid))
        .expect("3DSOLID must expose an exchange");
    let SolidExchange::Brep(brep) = exchange else {
        panic!("a valid SAT payload must lift to a neutral B-rep");
    };
    assert_eq!(brep.face_count(), 6);
    assert!(matches!(
        tess_brep(SolidExchange::Brep(brep)).outcome,
        TessellationOutcome::Success { .. }
    ));
}

#[test]
fn sab_payload_lifts_through_the_binary_reader() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::acis::SabWriter;
    use acadrust::entities::Solid3D;
    let doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
    let sab = SabWriter::write(&doc);
    let brep = sab_to_brep(&sab).expect("SAB must decode");
    assert_eq!(brep.face_count(), 6);
    let solid = Solid3D::from_sab(sab);
    let exchange = solid_exchange_from_entity(&EntityType::Solid3D(solid)).unwrap();
    assert!(matches!(exchange, SolidExchange::Brep(_)));
}

#[test]
fn cylinder_caps_and_side_tessellate_closed() {
    use acadrust::entities::acis::primitives::build_cylinder;
    let doc = build_cylinder([0.0, 0.0, 0.0], 1.0, 3.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 3, "two caps plus the side");
    assert!(brep.shells[0]
        .faces
        .iter()
        .any(|f| matches!(f.surface, BrepSurface::Cylinder { .. })));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            // The curved side makes it an approximation with a bound.
            assert!(matches!(
                geometry.precision,
                Precision::Approximate {
                    error_bound: Some(_)
                }
            ));
        }
        other => panic!("cylinder must be a closed Success, got {other:?}"),
    }
}

#[test]
fn sphere_lifts_to_one_loopless_face_and_tessellates() {
    use acadrust::entities::acis::primitives::build_sphere;
    let doc = build_sphere([0.0, 0.0, 0.0], 2.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 1);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() >= 8);
        }
        other => panic!("sphere must be a closed Success, got {other:?}"),
    }
}

#[test]
fn cone_lifts_to_a_base_cap_and_a_lateral_face_that_tessellates_closed() {
    use acadrust::entities::acis::primitives::build_cone;
    let doc = build_cone([0.0, 0.0, 0.0], 1.0, 2.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 2, "base cap plus lateral face");
    assert!(brep
        .shells
        .iter()
        .flat_map(|s| &s.faces)
        .any(|f| matches!(f.surface, BrepSurface::Cone { .. })));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0, "the cone must tessellate");
            // The curved lateral face makes it an approximation with a bound.
            assert!(matches!(
                geometry.precision,
                Precision::Approximate {
                    error_bound: Some(_)
                }
            ));
            // The planar base plus the lateral fan weld into a closed shell.
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "cone shell must be watertight");
        }
        other => panic!("cone must be a closed Success, got {other:?}"),
    }
}

#[test]
fn torus_lifts_to_one_loopless_face_and_tessellates_closed() {
    use acadrust::entities::acis::primitives::build_torus;
    let doc = build_torus([0.0, 0.0, 0.0], 3.0, 1.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 1);
    assert!(matches!(
        brep.shells[0].faces[0].surface,
        BrepSurface::Torus { .. }
    ));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            assert!(matches!(
                geometry.precision,
                Precision::Approximate {
                    error_bound: Some(_)
                }
            ));
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "torus seam must close");
        }
        other => panic!("torus must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_cone_is_a_closed_success() {
    let sat = include_str!("../../../fixtures/acis/cone.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 2);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "cone fixture must be watertight");
        }
        other => panic!("cone fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_truncated_cone_is_a_closed_success() {
    let sat = include_str!("../../../fixtures/acis/cone-truncated.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(
        brep.face_count(),
        3,
        "two circular caps plus the frustum side"
    );
    assert!(brep
        .shells
        .iter()
        .flat_map(|s| &s.faces)
        .any(|f| matches!(f.surface, BrepSurface::Cone { .. }) && f.loops.len() == 2));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "truncated cone fixture must be watertight");
        }
        other => panic!("truncated cone fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_torus_is_a_closed_success() {
    let sat = include_str!("../../../fixtures/acis/torus.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 1);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "torus fixture must be watertight");
        }
        other => panic!("torus fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_cube_parses_to_a_closed_six_face_solid() {
    let sat = include_str!("../../../fixtures/acis/cube.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 6);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert_eq!(geometry.triangle_count(), 12);
            let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
            assert!((area - 24.0).abs() < 1e-9, "area {area}");
        }
        other => panic!("cube fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_box_with_square_hole_tessellates_closed_with_holes() {
    let sat = include_str!("../../../fixtures/acis/box-with-square-hole.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 10, "two annuli plus eight walls");
    assert!(
        brep.shells
            .iter()
            .flat_map(|s| &s.faces)
            .any(|f| f.loops.len() == 2),
        "the annulus faces must carry an inner loop"
    );
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            // 2*(10*10) + 4*(10*4) - 2*(4*4) + 16*4 = 392.
            let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
            assert!((area - 392.0).abs() < 1e-9, "area {area}");
        }
        other => panic!("holed box must be a closed Success, got {other:?}"),
    }
}

#[test]
fn non_solid_entities_have_no_acis_exchange() {
    let line = EntityType::Line(acadrust::entities::Line::new());
    assert!(solid_exchange_from_entity(&line).is_none());
}

#[test]
fn region_body_and_surface_entities_share_the_acis_lift() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::{AcisData, Body, Region, Surface};
    let sat = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0).to_sat_string();
    let surface = Surface {
        acis_data: AcisData::from_sat(&sat),
        ..Surface::default()
    };
    let entities = [
        EntityType::Region(Region::from_sat(&sat)),
        EntityType::Body(Body::from_sat(&sat)),
        EntityType::Surface(surface),
    ];
    for entity in entities {
        match solid_exchange_from_entity(&entity) {
            Some(SolidExchange::Brep(brep)) => assert_eq!(brep.face_count(), 6),
            other => panic!("{entity:?} must lift to a B-rep, got {other:?}"),
        }
    }
}

#[test]
fn empty_acis_data_stays_raw_and_empty() {
    let empty = acadrust::entities::AcisData::new();
    let exchange = acis_exchange(&empty);
    assert!(exchange.is_empty());
    let result = tess_brep(exchange);
    assert!(matches!(result.outcome, TessellationOutcome::Failed { .. }));
}

#[test]
fn opaque_payload_retains_the_raw_acis_bytes() {
    let sat = "700 0 1 0\n@8 acadrust @8 ACIS 7.0 @24 Thu Jan 01 00:00:00 2023\n1e-06 1e-06\n-1 body $-1 $-1 $-1 $-1 #\n";
    let acis = acadrust::entities::AcisData::from_sat(sat);
    let (version, payload) = acis_raw_payload(&acis);
    assert_eq!(version, 1);
    assert!(!payload.is_empty());
}

// ---- plot settings (docs/plot.md) ----------------------------------------

/// Path to the committed synthetic plot fixture.
fn plot_fixture_bytes() -> Vec<u8> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn plot_fixture_imports_its_standalone_plot_settings() {
    // The fixture carries both embedded layout plot data (A4) and a standalone
    // PLOTSETTINGS object (A3, 90°). The object is the preferred source, so the
    // imported values must be the A3 ones, proven by the margins and rotation.
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(&request(plot_fixture_bytes()), &|| false)
        .expect("fixture imports");
    let layout = drawing
        .database
        .layouts()
        .next()
        .expect("fixture has a paper layout");
    let record = drawing
        .database
        .plot_settings(layout.id)
        .expect("standalone PLOTSETTINGS must be imported");
    assert_eq!(record.paper_size_name, "ISO_A3_(297.00_x_420.00_MM)");
    assert_eq!(record.paper_width, 297.0);
    assert_eq!(record.paper_height, 420.0);
    assert_eq!(record.rotation, cad_db::PlotRotation::Degrees90);
    assert_eq!(record.margins.left, 12.0);
    assert!(matches!(
        record.provenance,
        cad_db::PlotProvenance::Imported
    ));
    // The synthetic fixture has no model geometry; it must not claim Complete
    // render support for content it does not have. The import itself is valid.
    assert!(drawing.database.layout(layout.id).is_some());
}

#[test]
fn plot_fixture_without_plot_data_resolves_to_an_explicit_default() {
    // A layout id with no stored record must produce the documented A4 default,
    // never zero/unknown dimensions.
    let importer = AcadrustImporter::new();
    let drawing = importer
        .import(&request(plot_fixture_bytes()), &|| false)
        .expect("fixture imports");
    let fallback = drawing.database.plot_settings_for(LayoutId(999));
    assert!(matches!(
        fallback.provenance,
        cad_db::PlotProvenance::DefaultPage { .. }
    ));
    assert_eq!(fallback.paper_width, 210.0);
    assert_eq!(fallback.paper_height, 297.0);
}

// ---- Dynamic-block visibility mapping (spec §3.2) ----

use acadrust::objects::{BlockVisibilityParameter, BlockVisibilityState};
use std::collections::BTreeMap;

/// Build a synthetic acadrust document with a user block "DYN" owning three
/// line entities, governed by a two-state visibility parameter.
///
/// Entity 1 is visible, entity 2 is visible, entity 3 is invisible, so state
/// "A" (entities 1+2) is the baked-in active state. The returned handle is the
/// block record handle; the parameter owner chains to it.
/// Insert the three member entities a dynamic-visibility test block references.
///
/// The database builder validates that every block member exists, so the
/// mapping tests must give the definition real entity records.
fn insert_visibility_test_entities(builder: &mut DrawingDatabaseBuilder) {
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for id in [1u128, 2, 3] {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: Some(format!("{id:X}")),
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Block(BlockId(0)),
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: id as f64,
                        y: 0.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: id as f64,
                        y: 1.0,
                        z: 0.0,
                    },
                },
                draw_order: id as i64,
            })
            .unwrap();
    }
}

/// Build a synthetic acadrust document with a user block "DYN" owning three
/// line entities, governed by a two-state visibility parameter.
///
/// Entity 1 is visible, entity 2 is visible, entity 3 is invisible, so state
/// "A" (entities 1+2) is the baked-in active state. The returned handle is the
/// block record handle; the parameter owner chains to it.
fn synthetic_dynamic_document(
    active_is_b: bool,
) -> (
    acadrust::CadDocument,
    acadrust::Handle,
    Vec<acadrust::Handle>,
) {
    let mut doc = acadrust::CadDocument::new();
    let block_handle = doc.allocate_handle();
    let mut record = acadrust::tables::BlockRecord::new("DYN");
    record.handle = block_handle;
    doc.block_records.add(record).expect("add block record");

    let mut handles = Vec::new();
    for i in 0..3u64 {
        let mut line = acadrust::entities::Line::new();
        line.start = acadrust::types::Vector3::new(i as f64, 0.0, 0.0);
        line.end = acadrust::types::Vector3::new(i as f64, 1.0, 0.0);
        // State A ({handles[0], handles[1]}) hides handles[2]; state B
        // ({handles[0], handles[2]}) hides handles[1].
        line.common.invisible = if active_is_b { i == 1 } else { i == 2 };
        line.common.owner_handle = block_handle;
        let handle = doc
            .add_entity(EntityType::Line(line))
            .expect("add block member");
        handles.push(handle);
    }

    let param = BlockVisibilityParameter {
        handle: doc.allocate_handle(),
        owner: block_handle,
        all_blocks: handles.clone(),
        states: vec![
            BlockVisibilityState {
                name: "A".into(),
                visible_blocks: vec![handles[0], handles[1]],
                visible_params: Vec::new(),
            },
            BlockVisibilityState {
                name: "B".into(),
                visible_blocks: vec![handles[0], handles[2]],
                visible_params: Vec::new(),
            },
        ],
        ..BlockVisibilityParameter::default()
    };
    doc.block_visibility_params
        .insert(param.handle, param.clone());
    doc.objects.insert(
        param.handle,
        acadrust::objects::ObjectType::BlockVisibilityParameter(param),
    );
    (doc, block_handle, handles)
}

#[test]
fn importer_records_visibility_states_and_resolves_the_active_state() {
    let (doc, block_handle, handles) = synthetic_dynamic_document(false);
    let req = request(Vec::new());
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &doc, stats, compute_identity(&[]));

    // The importer assigns entity ids in walk order; the mapping keys on the
    // source handle values, so derive them from the synthetic document.
    let member_ids: BTreeMap<u64, EntityId> = [
        (handles[0].value(), EntityId(1)),
        (handles[1].value(), EntityId(2)),
        (handles[2].value(), EntityId(3)),
    ]
    .into_iter()
    .collect();
    // State A is baked in: entities 1 and 2 visible, entity 3 invisible.
    let member_visible: BTreeMap<u64, bool> = [
        (handles[0].value(), true),
        (handles[1].value(), true),
        (handles[2].value(), false),
    ]
    .into_iter()
    .collect();

    importer
        .builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    insert_visibility_test_entities(&mut importer.builder);
    importer.record_dynamic_visibility(block_handle, BlockId(0), &member_ids, &member_visible);

    let db = importer.builder.finish().unwrap();
    let visibility = db
        .block_dynamic_visibility(BlockId(0))
        .expect("a visibility descriptor is recorded");
    assert_eq!(visibility.state_names(), vec!["A", "B"]);
    assert_eq!(visibility.active_state.as_deref(), Some("A"));
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(1), EntityId(2)])
    );
}

#[test]
fn importer_records_state_b_when_its_flags_are_baked_in() {
    let (doc, block_handle, handles) = synthetic_dynamic_document(true);
    let req = request(Vec::new());
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &doc, stats, compute_identity(&[]));
    let member_ids: BTreeMap<u64, EntityId> = [
        (handles[0].value(), EntityId(1)),
        (handles[1].value(), EntityId(2)),
        (handles[2].value(), EntityId(3)),
    ]
    .into_iter()
    .collect();
    // State B: entities 1 and 3 visible, entity 2 invisible.
    let member_visible: BTreeMap<u64, bool> = [
        (handles[0].value(), true),
        (handles[1].value(), false),
        (handles[2].value(), true),
    ]
    .into_iter()
    .collect();
    importer
        .builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    insert_visibility_test_entities(&mut importer.builder);
    importer.record_dynamic_visibility(block_handle, BlockId(0), &member_ids, &member_visible);
    let db = importer.builder.finish().unwrap();
    let visibility = db.block_dynamic_visibility(BlockId(0)).unwrap();
    assert_eq!(visibility.active_state.as_deref(), Some("B"));
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(1), EntityId(3)])
    );
}

#[test]
fn ambiguous_visibility_flags_leave_the_active_state_unknown_and_partial() {
    // Neither state matches the actual flags (entity 1 hidden), so the active
    // state cannot be resolved and must stay unknown, with a stable reason.
    let param = BlockVisibilityParameter {
        all_blocks: vec![acadrust::Handle::new(1), acadrust::Handle::new(2)],
        states: vec![
            BlockVisibilityState {
                name: "A".into(),
                visible_blocks: vec![acadrust::Handle::new(1)],
                visible_params: Vec::new(),
            },
            BlockVisibilityState {
                name: "B".into(),
                visible_blocks: vec![acadrust::Handle::new(2)],
                visible_params: Vec::new(),
            },
        ],
        ..BlockVisibilityParameter::default()
    };
    let member_ids: BTreeMap<u64, EntityId> = [(1u64, EntityId(1)), (2, EntityId(2))]
        .into_iter()
        .collect();
    // Both members are invisible: no state (each makes one visible) matches,
    // so the active state is ambiguous and must stay unknown.
    let member_visible: BTreeMap<u64, bool> = [(1u64, false), (2, false)].into_iter().collect();
    let mapped = map_visibility(&param, &member_ids, &member_visible);
    assert!(mapped.descriptor.active_state.is_none());
    match mapped.completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r == REASON_ACTIVE_UNKNOWN),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    // No active state: every governed member stays visible, never a guess.
    assert!(mapped.descriptor.is_visible(EntityId(1)));
    assert!(mapped.descriptor.is_visible(EntityId(2)));
}

#[test]
fn dangling_visibility_member_handle_is_partial_with_a_stable_reason() {
    let param = BlockVisibilityParameter {
        all_blocks: vec![acadrust::Handle::new(1), acadrust::Handle::new(99)],
        states: vec![BlockVisibilityState {
            name: "A".into(),
            visible_blocks: vec![acadrust::Handle::new(1)],
            visible_params: Vec::new(),
        }],
        ..BlockVisibilityParameter::default()
    };
    // Handle 99 has no imported entity.
    let member_ids: BTreeMap<u64, EntityId> = [(1u64, EntityId(1))].into_iter().collect();
    let member_visible: BTreeMap<u64, bool> = [(1u64, true)].into_iter().collect();
    let mapped = map_visibility(&param, &member_ids, &member_visible);
    match mapped.completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r == REASON_MEMBER_UNRESOLVED),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn ungoverned_block_members_do_not_block_active_state_resolution() {
    // The block owns entity 3, which no state governs. It is visible and must
    // not make the active state ambiguous.
    let param = BlockVisibilityParameter {
        all_blocks: vec![acadrust::Handle::new(1), acadrust::Handle::new(2)],
        states: vec![
            BlockVisibilityState {
                name: "A".into(),
                visible_blocks: vec![acadrust::Handle::new(1)],
                visible_params: Vec::new(),
            },
            BlockVisibilityState {
                name: "B".into(),
                visible_blocks: vec![acadrust::Handle::new(2)],
                visible_params: Vec::new(),
            },
        ],
        ..BlockVisibilityParameter::default()
    };
    let member_ids: BTreeMap<u64, EntityId> = [
        (1u64, EntityId(1)),
        (2, EntityId(2)),
        (3, EntityId(3)), // ungoverned member
    ]
    .into_iter()
    .collect();
    // State A: entity 1 visible, entity 2 hidden; entity 3 (ungoverned) visible.
    let member_visible: BTreeMap<u64, bool> =
        [(1u64, true), (2, false), (3, true)].into_iter().collect();
    let mapped = map_visibility(&param, &member_ids, &member_visible);
    assert_eq!(mapped.descriptor.active_state.as_deref(), Some("A"));
    assert_eq!(mapped.completeness, Completeness::Complete);
    // The ungoverned entity is not part of the governed member set and stays
    // visible under any state.
    assert!(!mapped.descriptor.member_entities.contains(&EntityId(3)));
    assert!(mapped.descriptor.is_visible(EntityId(3)));
}

#[test]
fn importer_resolves_visibility_through_an_insert_reference() {
    let (mut doc, block_handle, handles) = synthetic_dynamic_document(false);
    // A model-space INSERT referencing the dynamic block, as a real evaluated
    // block reference does through its representation data.
    let mut insert = acadrust::entities::Insert::new("DYN", acadrust::types::Vector3::ZERO);
    insert.common.owner_handle = doc.header.model_space_block_handle;
    let insert_handle = doc
        .add_entity(EntityType::Insert(insert))
        .expect("add insert");
    let EntityType::Insert(insert) = doc.get_entity(insert_handle).unwrap().clone() else {
        panic!("just added an insert");
    };

    let req = request(Vec::new());
    let stats = ReadStats::default();
    let mut importer = ImporterBuilder::new(&req, &doc, stats, compute_identity(&[]));
    importer
        .builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
    insert_visibility_test_entities(&mut importer.builder);
    importer.block_ids.insert("DYN".into(), BlockId(0));
    importer.block_member_ids.insert(
        BlockId(0),
        [
            (handles[0].value(), EntityId(1)),
            (handles[1].value(), EntityId(2)),
            (handles[2].value(), EntityId(3)),
        ]
        .into_iter()
        .collect(),
    );
    importer.block_member_visible.insert(
        BlockId(0),
        [
            (handles[0].value(), true),
            (handles[1].value(), true),
            (handles[2].value(), false),
        ]
        .into_iter()
        .collect(),
    );

    importer.record_insert_dynamic_visibility(&insert);

    let db = importer.builder.finish().unwrap();
    let visibility = db
        .block_dynamic_visibility(BlockId(0))
        .expect("the INSERT path records the descriptor");
    assert_eq!(visibility.state_names(), vec!["A", "B"]);
    assert_eq!(visibility.active_state.as_deref(), Some("A"));
    // The parameter owner is the block record, not the INSERT; the resolver
    // still finds it because the INSERT references that block record.
    assert!(doc.block_visibility_param_for_def(block_handle).is_some());
}
