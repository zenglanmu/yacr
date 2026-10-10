//! Display-closure contract tests: MLINE joins/caps and TOLERANCE multi-line frame.

use super::*;

fn make_request() -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(Vec::<u8>::new().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    }
}

fn partial_reasons(completeness: &Completeness) -> Vec<String> {
    match completeness {
        Completeness::Partial(reasons) => reasons.clone(),
        other => panic!("expected Partial, got {other:?}"),
    }
}

fn v3(x: f64, y: f64, z: f64) -> acadrust::types::Vector3 {
    acadrust::types::Vector3::new(x, y, z)
}

// ---- PART 1: RASTERIMAGE emits an Image semantic ----

#[test]
fn raster_image_emits_image_semantic_with_placement_and_partial() {
    let mut img = acadrust::entities::RasterImage::new(
        "textures/photo.png",
        v3(1.0, 2.0, 0.0),
        1920.0,
        1080.0,
    );
    img.u_vector = v3(0.01, 0.0, 0.0);
    img.v_vector = v3(0.0, 0.02, 0.0);

    let (geometry, completeness) = raster_image_semantics(&img);
    let SemanticGeometry::Image {
        origin,
        u,
        v,
        pixels,
        file,
        clip,
        visible,
    } = geometry
    else {
        panic!("expected an image semantic, got {geometry:?}");
    };
    assert!((origin.x - 1.0).abs() < 1e-9 && (origin.y - 2.0).abs() < 1e-9);
    assert!((u.x - 0.01).abs() < 1e-9 && u.y.abs() < 1e-9);
    assert!((v.y - 0.02).abs() < 1e-9 && v.x.abs() < 1e-9);
    assert_eq!(pixels, [1920.0, 1080.0]);
    assert_eq!(file.as_deref(), Some("textures/photo.png"));
    assert!(clip.is_none(), "clipping is disabled by default");
    assert!(visible, "SHOW_IMAGE is set by RasterImage::new");

    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("resolved by the host/representation")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("frame is the fallback")),
        "{reasons:?}"
    );
}

#[test]
fn raster_image_maps_clip_and_treats_blank_file_as_none() {
    let mut img = acadrust::entities::RasterImage::new("   ", v3(0.0, 0.0, 0.0), 4.0, 4.0);
    // No SHOW_IMAGE flag, clipping enabled with an inside polygon boundary.
    img.flags = acadrust::entities::ImageDisplayFlags::USE_CLIPPING_BOUNDARY;
    img.clipping_enabled = true;
    img.clip_boundary = acadrust::entities::ClipBoundary {
        clip_type: acadrust::entities::ClipType::Polygonal,
        clip_mode: acadrust::entities::ClipMode::Inside,
        vertices: vec![
            acadrust::types::Vector2::new(0.0, 0.0),
            acadrust::types::Vector2::new(2.0, 0.0),
            acadrust::types::Vector2::new(2.0, 3.0),
        ],
    };

    let (geometry, completeness) = raster_image_semantics(&img);
    let SemanticGeometry::Image {
        file,
        clip,
        visible,
        ..
    } = geometry
    else {
        panic!("expected an image semantic, got {geometry:?}");
    };
    assert!(
        file.is_none(),
        "a blank path is a logical None, never a filesystem access"
    );
    assert!(!visible, "SHOW_IMAGE was cleared");
    let clip = clip.expect("clipping enabled maps the boundary");
    assert!(clip.inside, "ClipMode::Inside maps to inside == true");
    assert_eq!(clip.vertices, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 3.0]]);
    assert!(matches!(completeness, Completeness::Partial(_)));
}

// ---- PART 2: WIPEOUT emits the Mask semantic ----

#[test]
fn wipeout_emits_mask_with_boundary_and_inverted_flag() {
    let mut w = acadrust::entities::Wipeout::rectangular(v3(0.0, 0.0, 0.0), 10.0, 5.0);
    w.clip_mode = acadrust::entities::WipeoutClipMode::Inside;

    let (geometry, completeness) = wipeout_semantics(&w);
    let SemanticGeometry::Mask { boundary, inverted } = geometry else {
        panic!("expected a mask semantic, got {geometry:?}");
    };
    assert!(inverted, "clip mode Inside inverts the mask");
    assert_eq!(boundary.len(), 4, "{boundary:?}");
    assert!((boundary[1].x - 10.0).abs() < 1e-9, "{boundary:?}");
    assert!((boundary[2].y - 5.0).abs() < 1e-9, "{boundary:?}");

    let reasons = partial_reasons(&completeness);
    assert!(
        reasons.iter().any(|r| r.contains("mask boundary only")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("brightness/contrast/fade")),
        "{reasons:?}"
    );
}

#[test]
fn wipeout_polygonal_boundary_is_used_and_outside_is_not_inverted() {
    let mut w = acadrust::entities::Wipeout::rectangular(v3(0.0, 0.0, 0.0), 10.0, 5.0);
    // clip_mode defaults to Outside.
    w.clip_boundary_vertices = vec![
        acadrust::types::Vector2::new(0.0, 0.0),
        acadrust::types::Vector2::new(1.0, 0.0),
        acadrust::types::Vector2::new(1.0, 1.0),
    ];

    let (geometry, completeness) = wipeout_semantics(&w);
    let SemanticGeometry::Mask { boundary, inverted } = geometry else {
        panic!("expected a mask semantic, got {geometry:?}");
    };
    assert!(!inverted, "clip mode Outside is not inverted");
    assert_eq!(boundary.len(), 3, "{boundary:?}");
    // insertion + u*x + v*y for the first vertex (0,0) is the insertion point.
    assert!((boundary[0].x).abs() < 1e-9 && (boundary[0].y).abs() < 1e-9);
    assert!((boundary[1].x - 10.0).abs() < 1e-9);
    assert!((boundary[2].x - 10.0).abs() < 1e-9 && (boundary[2].y - 5.0).abs() < 1e-9);
    assert!(matches!(completeness, Completeness::Partial(_)));
}

// ---- PART 3: MLINE joins/caps from the style object ----

#[test]
fn mline_with_style_draws_elements_caps_and_joins() {
    let mut acad = acadrust::CadDocument::new();
    let handle = acad.allocate_handle();
    let mut style = acadrust::objects::MLineStyle::new("Custom");
    style.handle = handle;
    style
        .elements
        .push(acadrust::objects::MLineStyleElement::new(0.5));
    style
        .elements
        .push(acadrust::objects::MLineStyleElement::new(-0.5));
    style.flags.start_square_cap = true;
    style.flags.end_square_cap = true;
    style.flags.display_joints = true;
    acad.objects
        .insert(handle, acadrust::objects::ObjectType::MLineStyle(style));

    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut mline = acadrust::entities::MLine::new();
    mline.style_element_count = 2;
    mline.add_vertex(v3(0.0, 0.0, 0.0));
    mline.add_vertex(v3(10.0, 0.0, 0.0));
    mline.add_vertex(v3(10.0, 10.0, 0.0));
    mline.style_handle = Some(handle);

    let (geometry, completeness) = builder.mline_semantics(&mline);
    let SemanticGeometry::Compound(children) = &geometry else {
        panic!("expected a compound, got {geometry:?}");
    };
    // 2 element polylines + 2 square caps + 1 interior join.
    assert_eq!(children.len(), 5, "{children:?}");

    let reasons = partial_reasons(&completeness);
    assert!(
        reasons.iter().any(|r| r.contains("MLINESTYLE object")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("approximated")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("colours and linetypes are not carried")),
        "{reasons:?}"
    );
}

#[test]
fn mline_without_style_falls_back_and_stays_partial() {
    let mut mline = acadrust::entities::MLine::new();
    mline.style_element_count = 2;
    mline.add_vertex(v3(0.0, 0.0, 0.0));
    mline.add_vertex(v3(10.0, 0.0, 0.0));
    for vertex in mline.vertices.iter_mut() {
        if let Some(segment) = vertex.segments.first_mut() {
            segment.parameters = vec![0.5, -0.5];
        }
    }

    let acad = acadrust::CadDocument::new();
    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));
    let (geometry, completeness) = builder.mline_semantics(&mline);
    let SemanticGeometry::Compound(children) = &geometry else {
        panic!("expected a compound, got {geometry:?}");
    };
    // The two ±0.5 parameters must produce two parallel element lines at y ==
    // +0.5 and y == -0.5 (default justification is Zero, so the datum is 0).
    let element_ys: Vec<f64> = children
        .iter()
        .filter_map(|child| match child {
            SemanticGeometry::Polyline { points, .. } if points.len() >= 2 => {
                let y = points[0].y;
                points.iter().all(|p| (p.y - y).abs() < 1e-9).then_some(y)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        element_ys.len(),
        2,
        "expected two element polylines: {children:?}"
    );
    assert!(
        element_ys.iter().any(|y| (y - 0.5).abs() < 1e-9),
        "missing the +0.5 element: {element_ys:?}"
    );
    assert!(
        element_ys.iter().any(|y| (y + 0.5).abs() < 1e-9),
        "missing the -0.5 element: {element_ys:?}"
    );
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("the MLINESTYLE object was unavailable")),
        "{reasons:?}"
    );
}

#[test]
fn mline_enumerates_ignored_closed_caps_and_start_point() {
    let mut acad = acadrust::CadDocument::new();
    let handle = acad.allocate_handle();
    let mut style = acadrust::objects::MLineStyle::new("Custom");
    style.handle = handle;
    style
        .elements
        .push(acadrust::objects::MLineStyleElement::new(0.5));
    style
        .elements
        .push(acadrust::objects::MLineStyleElement::new(-0.5));
    acad.objects
        .insert(handle, acadrust::objects::ObjectType::MLineStyle(style));

    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut mline = acadrust::entities::MLine::closed_from_points(&[
        v3(0.0, 0.0, 0.0),
        v3(10.0, 0.0, 0.0),
        v3(10.0, 10.0, 0.0),
    ]);
    mline.style_handle = Some(handle);

    let (_geometry, completeness) = builder.mline_semantics(&mline);
    let reasons = partial_reasons(&completeness);
    assert!(reasons.iter().any(|r| r.contains("CLOSED")), "{reasons:?}");
    assert!(
        reasons.iter().any(|r| r.contains("start_angle/end_angle")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("start_point")),
        "{reasons:?}"
    );
}

#[test]
fn mline_justification_and_scale_shift_the_element_offsets() {
    let mut acad = acadrust::CadDocument::new();
    let handle = acad.allocate_handle();
    let mut style = acadrust::objects::MLineStyle::new("Custom");
    style.handle = handle;
    style
        .elements
        .push(acadrust::objects::MLineStyleElement::new(0.5));
    style
        .elements
        .push(acadrust::objects::MLineStyleElement::new(-0.5));
    acad.objects
        .insert(handle, acadrust::objects::ObjectType::MLineStyle(style));

    let req = make_request();
    let builder = ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));

    let mut mline = acadrust::entities::MLine::new();
    mline.style_element_count = 2;
    mline.add_vertex(v3(0.0, 0.0, 0.0));
    mline.add_vertex(v3(10.0, 0.0, 0.0));
    mline.style_handle = Some(handle);
    // Top justification puts the +0.5 element on the vertex line; scale 2
    // doubles the separation so the other element sits at -2 * (0.5 - -0.5)/2.
    mline.justification = acadrust::entities::MLineJustification::Top;
    mline.scale_factor = 2.0;

    let (geometry, _) = builder.mline_semantics(&mline);
    let SemanticGeometry::Compound(children) = &geometry else {
        panic!("expected a compound, got {geometry:?}");
    };
    // The top element (offset 0.5) is unshifted: it lies on y == 0.
    let top = children.iter().find_map(|child| match child {
        SemanticGeometry::Polyline { points, .. } => points
            .iter()
            .all(|p| p.y.abs() < 1e-9)
            .then_some(points.clone()),
        _ => None,
    });
    let top = top.expect("a top element polyline on the vertex line");
    assert!(top.iter().all(|p| p.y.abs() < 1e-9), "{top:?}");
    // The bottom element is 2 units below (offset difference 1 * scale 2).
    let bottom = children.iter().find_map(|child| match child {
        SemanticGeometry::Polyline { points, .. } => points
            .iter()
            .all(|p| (p.y + 2.0).abs() < 1e-9)
            .then_some(points.clone()),
        _ => None,
    });
    assert!(bottom.is_some(), "bottom element at y == -2: {children:?}");
}

// ---- PART 4: TOLERANCE multi-line frame ----

#[test]
fn tolerance_frame_height_reflects_the_line_count() {
    let mut tol = acadrust::entities::Tolerance::new();
    tol.text_height = 2.0;

    tol.text = "single".into();
    let (_, _, single_height) = tolerance_frame(&tol);
    assert!((single_height - 3.2).abs() < 1e-9, "{single_height}");

    tol.text = "a^Jb^Jc".into();
    assert_eq!(tol.line_count(), 3);
    let (geometry, width, multi_height) = tolerance_frame(&tol);
    assert!(
        (multi_height - 9.6).abs() < 1e-9,
        "three lines must be three times as tall: {multi_height}"
    );
    assert!(multi_height > single_height);
    // The frame is still a closed rectangle.
    let SemanticGeometry::Polyline { points, closed, .. } = geometry else {
        panic!("expected a polyline frame, got {geometry:?}");
    };
    assert!(closed);
    assert_eq!(points.len(), 4);
    // Width is sized from the widest line, not the whole string.
    assert!((width - (2.0 * 0.62 + 2.0)).abs() < 1e-9, "{width}");
}
