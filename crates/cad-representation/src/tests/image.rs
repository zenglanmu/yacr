use super::*;

#[test]
fn decoded_image_becomes_a_texture_primitive_mapping_the_unit_square() {
    let registry = ProviderRegistry::with_default_provider();
    let e = image_entity(1, Some("textures/Logo.PNG"), true);
    // The key is normalised, so a differently-cased path still resolves.
    let ctx = image_context(&[("textures/logo.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    assert_eq!(r.fragments.len(), 1);

    let primitive = &r.fragments[0].primitive;
    let DisplayPrimitive::Image {
        resource,
        transform,
        clip,
        ..
    } = primitive
    else {
        panic!("expected an image primitive");
    };
    assert_eq!(resource, &image_resource_key("textures/logo.png").unwrap());
    assert!(clip.is_none());

    let unit = |s: f64, t: f64| Point3 { x: s, y: t, z: 0.0 };
    // The unit square's corners map to the placed quad's corners.
    assert_point_eq(transform.apply_point(unit(0.0, 0.0)), p(1.0, 2.0));
    assert_point_eq(transform.apply_point(unit(1.0, 0.0)), p(3.0, 2.0));
    assert_point_eq(transform.apply_point(unit(0.0, 1.0)), p(1.0, 5.0));
    assert_point_eq(transform.apply_point(unit(1.0, 1.0)), p(3.0, 5.0));
    // The plane normal is the unit cross product of the two basis vectors.
    let base = transform.apply_point(unit(0.0, 0.0));
    let normal = transform.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    });
    assert_point_eq(
        Point3 {
            x: normal.x - base.x,
            y: normal.y - base.y,
            z: normal.z - base.z,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    );
}

#[test]
fn image_without_a_decoded_texture_draws_the_frame_and_reports_it() {
    let registry = ProviderRegistry::with_default_provider();
    let e = image_entity(2, Some("textures/missing.png"), true);
    // The cache holds a different key: the lookup must be by key, not "any image".
    let ctx = image_context(&[("textures/other.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert!(
        r.fragments
            .iter()
            .all(|f| !matches!(&f.primitive, DisplayPrimitive::Image { .. })),
        "no texture may be fabricated"
    );
    assert_eq!(r.fragments.len(), 1);
    let DisplayPrimitive::Lines(corners) = &r.fragments[0].primitive else {
        panic!("expected the frame as lines");
    };
    assert_eq!(corners.len(), 5, "a closed quad repeats its first corner");
    assert!(r.diagnostics.iter().any(|d| d.code == "image.unresolved"));
    assert!(matches!(r.completeness, Completeness::Partial(_)));
}

#[test]
fn image_with_a_rectangular_clip_maps_the_visible_polygon() {
    let registry = ProviderRegistry::with_default_provider();
    // Two opposite pixel-space corners of a full-image rectangle.
    let clip = ImageClip {
        vertices: vec![[0.0, 0.0], [2.0, 3.0]],
        inside: true,
    };
    let e = image_entity_with_clip(10, Some("clip.png"), Some(clip));
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    assert_eq!(r.fragments.len(), 1);

    let DisplayPrimitive::Image { clip, .. } = &r.fragments[0].primitive else {
        panic!("expected an image primitive");
    };
    let polygon = clip.as_ref().expect("clip is representable");
    assert_eq!(polygon.len(), 4, "a rectangle expands to four corners");
    let expected = [
        (p(1.0, 2.0), [0.0, 0.0]),
        (p(3.0, 2.0), [1.0, 0.0]),
        (p(3.0, 5.0), [1.0, 1.0]),
        (p(1.0, 5.0), [0.0, 1.0]),
    ];
    for (vertex, (position, uv)) in polygon.iter().zip(expected) {
        assert_point_eq(vertex.position, position);
        assert!((vertex.uv[0] - uv[0]).abs() < 1e-9 && (vertex.uv[1] - uv[1]).abs() < 1e-9);
    }
}

#[test]
fn a_clip_partly_outside_the_image_is_intersected_with_the_image_rect() {
    let registry = ProviderRegistry::with_default_provider();
    // `(-1,-1)..(1,1)` on a 2x3-pixel image: the image rectangle is the
    // intersection, so the emitted polygon is exactly `(0,0)..(1,1)` — no
    // out-of-image UV that the ClampToEdge sampler would smear.
    let clip = ImageClip {
        vertices: vec![[-1.0, -1.0], [1.0, 1.0]],
        inside: true,
    };
    let e = image_entity_with_clip(15, Some("clip.png"), Some(clip));
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    let DisplayPrimitive::Image { clip, .. } = &r.fragments[0].primitive else {
        panic!("expected an image primitive");
    };
    let polygon = clip.as_ref().expect("the clipped rect is representable");
    assert_eq!(polygon.len(), 4);
    let expected = [
        (p(1.0, 2.0), [0.0, 0.0]),
        (p(2.0, 2.0), [0.5, 0.0]),
        (p(2.0, 3.0), [0.5, 1.0 / 3.0]),
        (p(1.0, 3.0), [0.0, 1.0 / 3.0]),
    ];
    for (vertex, (position, uv)) in polygon.iter().zip(expected) {
        assert_point_eq(vertex.position, position);
        assert!((vertex.uv[0] - uv[0]).abs() < 1e-9 && (vertex.uv[1] - uv[1]).abs() < 1e-9);
        assert!((0.0..=1.0).contains(&vertex.uv[0]));
        assert!((0.0..=1.0).contains(&vertex.uv[1]));
    }
}

#[test]
fn a_clip_fully_outside_the_image_reports_unsupported_and_draws_the_frame() {
    let registry = ProviderRegistry::with_default_provider();
    let clip = ImageClip {
        vertices: vec![[5.0, 5.0], [6.0, 6.0]],
        inside: true,
    };
    let e = image_entity_with_clip(16, Some("clip.png"), Some(clip));
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert!(
        r.fragments
            .iter()
            .all(|f| !matches!(&f.primitive, DisplayPrimitive::Image { .. })),
        "a fully clipped-away rect must not draw stretched edge texels"
    );
    assert!(matches!(
        &r.fragments[0].primitive,
        DisplayPrimitive::Lines(_)
    ));
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "image.clip_unsupported"));
    assert!(matches!(r.completeness, Completeness::Partial(_)));
}

#[test]
fn an_inside_clip_rect_is_unchanged_by_the_image_intersection() {
    let registry = ProviderRegistry::with_default_provider();
    let clip = ImageClip {
        vertices: vec![[0.5, 0.5], [1.5, 2.5]],
        inside: true,
    };
    let e = image_entity_with_clip(17, Some("clip.png"), Some(clip));
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    let DisplayPrimitive::Image { clip, .. } = &r.fragments[0].primitive else {
        panic!("expected an image primitive");
    };
    let polygon = clip.as_ref().expect("an inside rect is representable");
    let expected = [
        (p(1.5, 2.5), [0.25, 0.5 / 3.0]),
        (p(2.5, 2.5), [0.75, 0.5 / 3.0]),
        (p(2.5, 4.5), [0.75, 2.5 / 3.0]),
        (p(1.5, 4.5), [0.25, 2.5 / 3.0]),
    ];
    assert_eq!(polygon.len(), 4);
    for (vertex, (position, uv)) in polygon.iter().zip(expected) {
        assert_point_eq(vertex.position, position);
        assert!((vertex.uv[0] - uv[0]).abs() < 1e-9 && (vertex.uv[1] - uv[1]).abs() < 1e-9);
    }
}

#[test]
fn image_with_a_closed_polygon_clip_drops_the_repeated_vertex() {
    let registry = ProviderRegistry::with_default_provider();
    let clip = ImageClip {
        // The importer stores a closed polygon; the primitive polygon is open.
        vertices: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 3.0], [0.0, 3.0], [0.0, 0.0]],
        inside: true,
    };
    let e = image_entity_with_clip(11, Some("clip.png"), Some(clip));
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    let DisplayPrimitive::Image { clip, .. } = &r.fragments[0].primitive else {
        panic!("expected an image primitive");
    };
    assert_eq!(clip.as_ref().expect("clip is representable").len(), 4);
}

#[test]
fn convex_polygon_clip_yields_the_image_primitive() {
    let registry = ProviderRegistry::with_default_provider();
    // A convex non-rectangular pentagon (within the 2x3 pixel frame).
    let clip = ImageClip {
        vertices: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 3.0], [0.0, 2.0]],
        inside: true,
    };
    let e = image_entity_with_clip(13, Some("clip.png"), Some(clip));
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    let DisplayPrimitive::Image { clip, .. } = &r.fragments[0].primitive else {
        panic!("expected an image primitive");
    };
    assert_eq!(
        clip.as_ref().expect("a convex clip is representable").len(),
        5
    );
}

#[test]
fn non_convex_polygon_clip_falls_back_to_the_frame_and_reports_it() {
    let registry = ProviderRegistry::with_default_provider();
    // A concave "chevron": the turn at (1,1) reverses the edge cross-product
    // sign, so the polygon is non-convex and the renderer's fan triangulation
    // would fill a silently wrong region.
    let clip = ImageClip {
        vertices: vec![[0.0, 0.0], [2.0, 0.0], [1.0, 1.0], [2.0, 2.0], [0.0, 2.0]],
        inside: true,
    };
    let e = image_entity_with_clip(14, Some("clip.png"), Some(clip));
    // The texture is available, but the non-convex clip cannot be honoured.
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert!(
        r.fragments
            .iter()
            .all(|f| !matches!(&f.primitive, DisplayPrimitive::Image { .. })),
        "a non-convex clip must not draw the texture"
    );
    assert_eq!(r.fragments.len(), 1);
    assert!(matches!(
        &r.fragments[0].primitive,
        DisplayPrimitive::Lines(_)
    ));
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "image.clip_unsupported"));
    assert!(matches!(r.completeness, Completeness::Partial(_)));
}

#[test]
fn outside_clip_falls_back_to_the_frame_and_reports_it() {
    let registry = ProviderRegistry::with_default_provider();
    // `inside == false` keeps the region outside the boundary; the
    // visible-region polygon cannot express that.
    let clip = ImageClip {
        vertices: vec![[0.0, 0.0], [2.0, 3.0]],
        inside: false,
    };
    let e = image_entity_with_clip(12, Some("clip.png"), Some(clip));
    // The texture is available, but the clip cannot be honoured.
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert!(
        r.fragments
            .iter()
            .all(|f| !matches!(&f.primitive, DisplayPrimitive::Image { .. })),
        "an unrepresentable clip must not draw the texture"
    );
    assert_eq!(r.fragments.len(), 1);
    assert!(matches!(
        &r.fragments[0].primitive,
        DisplayPrimitive::Lines(_)
    ));
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "image.clip_unsupported"));
    assert!(matches!(r.completeness, Completeness::Partial(_)));
}

#[test]
fn image_without_a_resource_key_is_reported_unresolved() {
    let registry = ProviderRegistry::with_default_provider();
    let e = image_entity(3, None, true);
    let r = registry.build(&e, &context()).unwrap();
    assert!(r.diagnostics.iter().any(|d| d.code == "image.unresolved"));
    assert_eq!(r.fragments.len(), 1);
    assert!(matches!(
        &r.fragments[0].primitive,
        DisplayPrimitive::Lines(_)
    ));
}

#[test]
fn hidden_image_emits_no_fragment() {
    let registry = ProviderRegistry::with_default_provider();
    let e = image_entity(4, Some("textures/logo.png"), false);
    let ctx = image_context(&[("textures/logo.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert!(r.fragments.is_empty());
    assert!(r.diagnostics.is_empty());
    assert_eq!(r.completeness, Completeness::Complete);
}

/// The per-image display transfer is carried on the primitive and is
/// view-independent: a transform copies it unchanged.
#[test]
fn image_adjustment_reaches_the_primitive_and_survives_a_transform() {
    let registry = ProviderRegistry::with_default_provider();
    let mut e = image_entity(9, Some("textures/logo.png"), true);
    let adjust = ImageAdjust {
        brightness: 30,
        contrast: 70,
        fade: 20,
        monochrome: true,
    };
    match &mut e.geometry {
        SemanticGeometry::Image { adjust: slot, .. } => *slot = adjust,
        _ => panic!("expected an image"),
    }
    let ctx = image_context(&[("textures/logo.png", white_pixel())]);
    let r = registry.build(&e, &ctx).unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    let DisplayPrimitive::Image {
        adjust: carried, ..
    } = &r.fragments[0].primitive
    else {
        panic!("expected an image primitive");
    };
    assert_eq!(*carried, adjust, "the provider copies the entity transfer");

    let moved = r.fragments[0]
        .primitive
        .transformed(&Transform3::translation(p(10.0, 5.0)));
    let DisplayPrimitive::Image {
        adjust: after_move, ..
    } = moved
    else {
        panic!("expected an image primitive after the transform");
    };
    assert_eq!(
        after_move, adjust,
        "brightness/contrast/fade/monochrome are display-only, never transformed"
    );
}
