//! Mesh and line batch construction and topology.

use super::*;

#[test]
fn mesh_batch_is_indexed_and_carries_normals_and_edges() {
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Mesh);
    // Indexed: one vertex per mesh vertex, not per triangle corner.
    assert_eq!(batch.vertices.len(), 4);
    assert_eq!(batch.normals.len(), 4);
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(batch.triangle_count(), 2);
    // The overlay is the surface **boundary**: the diagonal (shared by both
    // triangles) cancels, so a quad has 4 boundary segments => 8 positions.
    assert_eq!(batch.edges.len(), 8);
}

#[test]
fn quad_overlay_is_the_surface_boundary_without_the_diagonal() {
    // The overlay is the mesh's surface boundary (odd-use rule), not every
    // triangle edge: the diagonal shared by the quad's two triangles is not an
    // edge of the surface and must never be drawn as one.
    let delta = SceneCache::default()
        .build(&mesh_representation(6, quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(
        batch.edges.len(),
        8,
        "a quad has four boundary segments, not five triangle edges"
    );
    let segments: Vec<([f32; 3], [f32; 3])> = batch
        .edges
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0], pair[1]))
        .collect();
    let has_segment = |a: [f32; 3], b: [f32; 3]| {
        segments
            .iter()
            .any(|&(x, y)| (x == a && y == b) || (x == b && y == a))
    };
    // The diagonal is v0=(0,0,0) → v2=(1,1,0).
    assert!(
        !has_segment([0.0, 0.0, 0.0], [1.0, 1.0, 0.0]),
        "the tessellation diagonal must not be drawn as an edge"
    );
    // Every outer edge of the quad is present.
    for (a, b) in [
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ([1.0, 0.0, 0.0], [1.0, 1.0, 0.0]),
        ([1.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 0.0]),
    ] {
        assert!(has_segment(a, b), "missing boundary segment {a:?} -> {b:?}");
    }
}

#[test]
fn a_mesh_with_only_shared_edges_has_no_boundary_and_is_not_a_text_glyph() {
    // A closed mesh (every edge used by both triangles) has an empty boundary.
    // That is the same edge list a text glyph has, so the explicit
    // `text_glyph` field — not an empty `edges` — must decide the class;
    // otherwise an imported closed mesh would vanish during camera motion.
    let mut mesh = quad();
    mesh.triangles = vec![[0, 1, 2], [0, 1, 2]];
    let delta = SceneCache::default()
        .build(&mesh_representation(7, mesh), stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert!(
        batch.edges.is_empty(),
        "every edge is used twice, so nothing is a boundary"
    );
    assert!(!batch.text_glyph, "a closed mesh is not a shaped glyph");
    assert!(!batch.is_text_glyph());
}

#[test]
fn text_glyph_is_explicit_and_stored_edges_are_never_text() {
    // `MeshWithoutEdgeOverlay` is the only primitive that sets the explicit
    // class; it stays true even though the batch also carries no edges.
    let mut glyph_rep = mesh_representation(8, quad());
    glyph_rep.fragments[0].primitive = DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(quad()));
    let glyph_delta = SceneCache::default().build(&glyph_rep, stamp()).unwrap();
    let glyph = &glyph_delta.added[0];
    assert!(glyph.edges.is_empty());
    assert!(glyph.text_glyph, "the glyph class is set explicitly");
    assert!(glyph.is_text_glyph());

    // A body's stored edges are the wireframe source and the batch is never the
    // text class.
    let mesh = quad();
    let stored = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let mut body_rep = mesh_representation(9, mesh.clone());
    body_rep.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
        derive_when_empty: true,
    };
    let body_delta = SceneCache::default().build(&body_rep, stamp()).unwrap();
    let body = &body_delta.added[0];
    assert_eq!(
        body.edges,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        "the stored segment is the overlay, not the boundary"
    );
    assert!(!body.text_glyph);
    assert!(!body.is_text_glyph());
}

#[test]
fn filled_mesh_can_disable_its_edge_overlay_without_changing_triangle_topology() {
    let mut representation = mesh_representation(2, quad());
    representation.fragments[0].primitive =
        DisplayPrimitive::MeshWithoutEdgeOverlay(std::sync::Arc::new(quad()));
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert!(
        batch.edges.is_empty(),
        "disabled mesh overlay must not allocate edge geometry"
    );
    assert_eq!(batch.upload_size_bytes(), 4 * 12 * 3 + 2 * 12);
}

#[test]
fn text_glyph_class_is_an_indexed_mesh_without_edge_overlay() {
    // The glyph primitive (no per-edge overlay) is the text display class.
    let mut glyph_rep = mesh_representation(3, quad());
    glyph_rep.fragments[0].primitive = DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(quad()));
    let glyph_delta = SceneCache::default().build(&glyph_rep, stamp()).unwrap();
    let glyph = &glyph_delta.added[0];
    assert!(
        glyph.is_text_glyph(),
        "indexed mesh with no edges is a glyph"
    );

    // A normal mesh carries the wireframe overlay: not text.
    let normal_delta = SceneCache::default()
        .build(&mesh_representation(4, quad()), stamp())
        .unwrap();
    let normal = &normal_delta.added[0];
    assert!(!normal.edges.is_empty());
    assert!(
        !normal.is_text_glyph(),
        "a mesh with edge overlay is not text"
    );

    // A zero-triangle mesh has no indices (and no edges): not text.
    let mut empty_mesh = quad();
    empty_mesh.triangles.clear();
    let empty_delta = SceneCache::default()
        .build(&mesh_representation(5, empty_mesh), stamp())
        .unwrap();
    let empty = &empty_delta.added[0];
    assert!(empty.indices.is_empty());
    assert!(!empty.is_text_glyph(), "a zero-triangle mesh is not text");
}

#[test]
fn body_mesh_batch_uses_its_stored_edges_not_the_triangle_derived_ones() {
    // A `Body3d` with a display mesh and stored edges arrives as
    // `DisplayPrimitive::MeshWithStoredEdges`. Its batch must carry the stored
    // endpoint pairs, never the twelve triangle-derived positions of the quad.
    let mesh = quad();
    let stored = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let mut representation = mesh_representation(20, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
        derive_when_empty: true,
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Mesh);
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(
        batch.edges,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        "one stored segment, relative to the batch origin; not 12 triangle edges"
    );
    assert!(!batch.is_text_glyph());
}

#[test]
fn body_stored_edges_are_deduplicated_and_degenerate_segments_dropped() {
    let mesh = quad();
    let a = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let b = Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    let c = Point3 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
    };
    let non_finite = Point3 {
        x: f64::NAN,
        y: 0.0,
        z: 0.0,
    };
    let stored = vec![
        a, b, // first occurrence, kept
        b, a, // same unordered pair in the other direction: dropped
        c, c, // zero length: dropped
        non_finite, b, // non-finite endpoint: dropped
        b, c, // a distinct segment, kept
    ];
    let mut representation = mesh_representation(21, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
        derive_when_empty: true,
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(
        batch.edges,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0]
        ],
        "first occurrence of each unordered segment, in input order"
    );
}

#[test]
fn body_stored_edges_are_relative_to_the_batch_local_origin() {
    let mut mesh = quad();
    // Shift every vertex so the batch origin is the first vertex, not (0,0,0).
    for vertex in &mut mesh.vertices {
        vertex.x += 10.0;
        vertex.y += 20.0;
        vertex.z += 30.0;
    }
    let origin = mesh.vertices[0];
    let stored = vec![
        origin,
        Point3 {
            x: origin.x + 2.0,
            y: origin.y,
            z: origin.z,
        },
    ];
    let mut representation = mesh_representation(22, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
        derive_when_empty: true,
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.local_origin, origin);
    assert_eq!(batch.edges, vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
}

#[test]
fn mesh_with_stored_edges_but_no_usable_segment_keeps_the_triangle_overlay() {
    // Defensive only: the provider never emits an empty stored-edge list, but if
    // one reaches the scene the legacy triangle-derived overlay is kept (and the
    // batch is still not a text glyph).
    let mesh = quad();
    let mut representation = mesh_representation(23, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(Vec::<Point3>::new().into_boxed_slice()),
        derive_when_empty: true,
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.edges.len(), 10);
    assert!(!batch.is_text_glyph());
}

#[test]
fn body_edges_only_fragment_becomes_a_line_batch_with_no_triangles() {
    // A zero-triangle body with non-empty edges arrives as `LineSegments`
    // (consecutive segment endpoint pairs), which builds as a line batch. It
    // never reaches the index-less `RenderTopology::Mesh` path whose
    // `triangle_count` fallback would read `vertices.len() / 3` as invented
    // triangles.
    let mut representation = line_representation(21, unit_points());
    let mut edges = unit_points();
    edges.extend([
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
    ]);
    representation.fragments[0].primitive =
        DisplayPrimitive::LineSegments(Arc::from(edges.into_boxed_slice()));
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
    // Two endpoint pairs are four vertices, and none are joined into a polyline.
    assert_eq!(batch.vertices.len(), 4);
}

#[test]
fn body_wireframe_edges_only_fragment_is_a_line_batch_with_no_triangles() {
    // The `VSCURRENT WIREFRAME` body path emits `LineSegments` (stored edges or
    // unique triangle-derived edges). Whatever the source, the scene must build
    // a line batch: no indices, no triangles, and one vertex per endpoint.
    let segments = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
    ];
    let mut representation = line_representation(24, unit_points());
    representation.fragments[0].primitive =
        DisplayPrimitive::LineSegments(Arc::from(segments.into_boxed_slice()));
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
    assert_eq!(
        batch.vertices.len(),
        6,
        "three endpoint pairs, not a polyline"
    );
}

#[test]
fn lines_batch_has_no_triangles_or_normals() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.normals.is_empty());
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
}

#[test]
fn line_topologies_do_not_consume_the_triangle_budget() {
    let mut cache = SceneCache::default();
    let mut points = unit_points();
    points.extend(unit_points());
    points.extend(unit_points());
    let delta = cache
        .build(&line_representation(1, points), stamp())
        .unwrap();
    let mut batch = delta.added[0].clone();
    let budget = FrameBudget {
        max_vertices: 6,
        max_triangles: 0,
        max_bytes: usize::MAX,
    };
    for topology in [RenderTopology::Lines, RenderTopology::MeshEdges] {
        batch.topology = topology;
        assert_eq!(batch.triangle_count(), 0);
        let mut usage = FrameUsage::default();
        budget
            .charge(&mut usage, batch.vertices.len(), batch.triangle_count())
            .unwrap();
        assert_eq!(usage.vertices, 6);
        assert_eq!(usage.triangles, 0);
    }
    // Non-indexed mesh batches retain their triangle-list fallback.
    batch.topology = RenderTopology::Mesh;
    assert_eq!(batch.triangle_count(), 2);
}

// ---- UV0 / base-colour texture (ADR-0038 D8) ----------------------------

/// A canonical document texture key (`<32 lowercase hex>.png`).
const TEXTURE_KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

fn textured_quad() -> Mesh {
    let mut mesh = quad();
    mesh.uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    mesh.texture = Some(TEXTURE_KEY.to_owned());
    mesh
}

#[test]
fn mesh_batch_carries_uvs_and_a_resource_key_for_a_textured_mesh() {
    let delta = SceneCache::default()
        .build(&mesh_representation(1, textured_quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(
        batch.uvs,
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
    );
    assert_eq!(batch.uvs.len(), batch.vertices.len());
    assert_eq!(
        batch.texture.as_ref().map(|key| key.as_str()),
        Some(TEXTURE_KEY),
        "the domain String key is converted to a ResourceKey at the scene boundary"
    );
}

#[test]
fn untextured_mesh_uploads_no_uv_stream_even_when_it_carries_uvs() {
    // UVs without a texture cannot be sampled by any pipeline, so the batch
    // keeps the untextured form: both fields empty. This is a documented
    // sanitisation, not a silent loss of a drawable attribute.
    let mut mesh = quad();
    mesh.uvs = vec![[0.0, 0.0]; 4];
    let delta = SceneCache::default()
        .build(&mesh_representation(1, mesh), stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert!(batch.uvs.is_empty());
    assert!(batch.texture.is_none());
    assert_eq!(
        batch.upload_size_bytes(),
        4 * 12 * 3 + 2 * 12 + 8 * 12,
        "no UV bytes are charged for an untextured batch"
    );
}

#[test]
fn a_texture_with_misaligned_or_invalid_uvs_is_sanitised_not_panicked() {
    // Wrong count: the domain requires one UV per vertex.
    let mut wrong_count = quad();
    wrong_count.uvs = vec![[0.0, 0.0], [1.0, 0.0]];
    wrong_count.texture = Some(TEXTURE_KEY.to_owned());
    let delta = SceneCache::default()
        .build(&mesh_representation(1, wrong_count), stamp())
        .unwrap();
    assert!(delta.added[0].uvs.is_empty());
    assert!(delta.added[0].texture.is_none());

    // A non-finite UV can never reach the GPU as NaN.
    let mut nan_uv = quad();
    nan_uv.uvs = vec![[f64::NAN, 0.0]; 4];
    nan_uv.texture = Some(TEXTURE_KEY.to_owned());
    let delta = SceneCache::default()
        .build(&mesh_representation(2, nan_uv), stamp())
        .unwrap();
    assert!(delta.added[0].uvs.is_empty());
    assert!(delta.added[0].texture.is_none());

    // An unsafe key is refused by the same defensive filter (cad-db validation
    // is the authority; the scene is the last line).
    let mut bad_key = quad();
    bad_key.uvs = vec![[0.0, 0.0]; 4];
    bad_key.texture = Some("../escape.png".to_owned());
    let delta = SceneCache::default()
        .build(&mesh_representation(3, bad_key), stamp())
        .unwrap();
    assert!(delta.added[0].uvs.is_empty());
    assert!(delta.added[0].texture.is_none());
}

#[test]
fn textured_mesh_upload_bytes_and_cpu_bytes_charge_the_uv_stream() {
    let delta = SceneCache::default()
        .build(&mesh_representation(1, textured_quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    // positions + normals + colours (4 × 12 each) + indices (2 × 12) + edges
    // (8 × 12: the 4 boundary segments) + UVs (4 × 8).
    assert_eq!(
        batch.upload_size_bytes(),
        4 * 12 * 3 + 2 * 12 + 8 * 12 + 4 * 8
    );
    assert_eq!(batch.vertex_bytes(), 4 * 12 * 2 + 8 * 12 + 4 * 8);
    // `approx_bytes` counts the streams the cache actually holds (the batch has
    // no per-vertex colour stream) plus the UV0 stream.
    assert_eq!(batch.approx_bytes(), (4 + 4 + 8) * 12 + 2 * 12 + 4 * 8);
}

#[test]
fn mesh_texture_keys_are_deduplicated_and_sorted() {
    let other_key = "0f1e2d3c4b5a69788796a5b4c3d2e1f0.jpg";
    let mut second = quad();
    second.uvs = vec![[0.0, 0.0]; 4];
    second.texture = Some(other_key.to_owned());
    let representation = DisplayRepresentation {
        fragments: [
            mesh_representation(1, textured_quad()),
            mesh_representation(2, textured_quad()),
            mesh_representation(3, second),
            mesh_representation(4, quad()),
        ]
        .into_iter()
        .flat_map(|rep| rep.fragments)
        .collect(),
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    };
    let mut cache = SceneCache::default();
    let delta = cache.build(&representation, stamp()).unwrap();
    let keys = mesh_texture_keys(delta.added.iter());
    assert_eq!(
        keys.iter().map(|key| key.as_str()).collect::<Vec<_>>(),
        vec![other_key, TEXTURE_KEY],
        "untextured batches contribute nothing; keys are deduped and sorted"
    );
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.mesh_texture_keys(), keys);
}

// ---- PBR channels (ADR-0039 D6) -----------------------------------------

const MR_KEY: &str = "11111111111111111111111111111111.png";
const NORMAL_KEY: &str = "22222222222222222222222222222222.jpg";
const EMISSIVE_KEY: &str = "33333333333333333333333333333333.png";
const OCCLUSION_KEY: &str = "44444444444444444444444444444444.png";

fn pbr_material() -> cad_domain::MeshPbr {
    cad_domain::MeshPbr {
        metallic: 0.25,
        roughness: 0.5,
        emissive: [0.5, 0.25, 0.0],
        occlusion_strength: 0.75,
        metallic_roughness_texture: Some(MR_KEY.to_owned()),
        normal_texture: Some(NORMAL_KEY.to_owned()),
        emissive_texture: Some(EMISSIVE_KEY.to_owned()),
        occlusion_texture: Some(OCCLUSION_KEY.to_owned()),
    }
}

fn pbr_quad() -> Mesh {
    let mut mesh = textured_quad();
    mesh.pbr = Some(pbr_material());
    mesh
}

#[test]
fn mesh_batch_carries_the_converted_pbr_material() {
    let delta = SceneCache::default()
        .build(&mesh_representation(1, pbr_quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    let pbr = batch
        .pbr
        .as_ref()
        .expect("the material survives conversion");
    assert_eq!(pbr.metallic, 0.25);
    assert_eq!(pbr.roughness, 0.5);
    assert_eq!(pbr.emissive, [0.5, 0.25, 0.0]);
    assert_eq!(pbr.occlusion_strength, 0.75);
    assert_eq!(
        pbr.metallic_roughness.as_ref().map(|k| k.as_str()),
        Some(MR_KEY)
    );
    assert_eq!(pbr.normal.as_ref().map(|k| k.as_str()), Some(NORMAL_KEY));
    assert_eq!(
        pbr.emissive_texture.as_ref().map(|k| k.as_str()),
        Some(EMISSIVE_KEY)
    );
    assert_eq!(
        pbr.occlusion.as_ref().map(|k| k.as_str()),
        Some(OCCLUSION_KEY)
    );
    assert_eq!(
        batch.texture.as_ref().map(|k| k.as_str()),
        Some(TEXTURE_KEY)
    );
    assert_eq!(batch.uvs.len(), batch.vertices.len());
}

#[test]
fn a_pbr_material_with_bad_or_out_of_range_factors_is_dropped_whole() {
    for break_it in [
        |pbr: &mut cad_domain::MeshPbr| pbr.metallic = 1.5,
        |pbr: &mut cad_domain::MeshPbr| pbr.roughness = f64::NAN,
        |pbr: &mut cad_domain::MeshPbr| pbr.occlusion_strength = -0.1,
        |pbr: &mut cad_domain::MeshPbr| pbr.emissive = [0.0, f64::INFINITY, 0.0],
    ] {
        let mut mesh = pbr_quad();
        let mut pbr = pbr_material();
        break_it(&mut pbr);
        mesh.pbr = Some(pbr);
        let delta = SceneCache::default()
            .build(&mesh_representation(1, mesh), stamp())
            .unwrap();
        assert!(
            delta.added[0].pbr.is_none(),
            "an unrepresentable factor drops the whole material, never a clamped guess"
        );
        // The base-colour texture channel is independent and still drawable.
        assert!(delta.added[0].texture.is_some());
    }
}

#[test]
fn a_pbr_texture_without_uvs_is_dropped_but_the_scalar_factors_survive() {
    let mut mesh = quad();
    // No UV stream at all: no PBR map can be sampled from UV0.
    mesh.pbr = Some(pbr_material());
    let delta = SceneCache::default()
        .build(&mesh_representation(1, mesh), stamp())
        .unwrap();
    let batch = &delta.added[0];
    let pbr = batch
        .pbr
        .as_ref()
        .expect("the scalar factors are UV-independent and survive");
    assert_eq!(pbr.metallic, 0.25);
    assert!(
        !pbr.has_textures(),
        "every PBR texture slot needs a UV stream; all four are dropped"
    );
    assert!(batch.uvs.is_empty());

    // A misaligned UV stream drops the maps the same way.
    let mut misaligned = pbr_quad();
    misaligned.uvs = vec![[0.0, 0.0], [1.0, 0.0]];
    let delta = SceneCache::default()
        .build(&mesh_representation(2, misaligned), stamp())
        .unwrap();
    let pbr = delta.added[0].pbr.as_ref().expect("scalars survive");
    assert!(!pbr.has_textures());
    assert!(delta.added[0].uvs.is_empty());

    // An unsafe PBR key is refused by the same defensive filter.
    let mut bad_key = pbr_quad();
    bad_key.pbr.as_mut().unwrap().normal_texture = Some("../escape.png".to_owned());
    let delta = SceneCache::default()
        .build(&mesh_representation(3, bad_key), stamp())
        .unwrap();
    let pbr = delta.added[0].pbr.as_ref().expect("scalars survive");
    assert!(pbr.normal.is_none(), "an unsafe key never reaches the GPU");
    assert!(pbr.metallic_roughness.is_some());
}

#[test]
fn pbr_material_bytes_charge_the_keys_and_the_tangent_stream() {
    let delta = SceneCache::default()
        .build(&mesh_representation(1, pbr_quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    let pbr = batch.pbr.as_ref().expect("material");
    let key_bytes = [MR_KEY, NORMAL_KEY, EMISSIVE_KEY, OCCLUSION_KEY]
        .iter()
        .map(|key| key.len())
        .sum::<usize>();
    // The normal map needs a `Float32x4` tangent per vertex.
    let tangent_bytes = batch.vertices.len() * 16;
    assert_eq!(pbr.key_bytes(), key_bytes);
    // ADR-0044: a PBR mesh emits no triangle-derived edge overlay, so the
    // geometry term is positions + normals + colours + indices + UV0 only.
    let geometry = 4 * 12 * 3 + 2 * 12 + 4 * 8;
    assert_eq!(
        batch.upload_size_bytes(),
        geometry + key_bytes + tangent_bytes,
        "a PBR batch uploads its tangent stream and charges its key strings"
    );
    assert_eq!(
        batch.vertex_bytes(),
        4 * 12 * 2 + 4 * 8 + key_bytes + tangent_bytes
    );
    assert_eq!(
        batch.approx_bytes(),
        (4 + 4) * 12 + 2 * 12 + 4 * 8 + key_bytes + tangent_bytes
    );

    // A material without a normal map needs no tangent frame.
    let mut scalar_only = quad();
    scalar_only.pbr = Some(cad_domain::MeshPbr {
        metallic: 1.0,
        roughness: 0.2,
        ..cad_domain::MeshPbr::default()
    });
    let delta = SceneCache::default()
        .build(&mesh_representation(2, scalar_only), stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert!(
        !batch.pbr.as_ref().expect("material").needs_tangents(),
        "no normal map, no tangent frame"
    );
    // With no texture channel active, nothing samples UV0, so the batch keeps
    // the untextured form and uploads no UV stream either. Its PBR material is
    // still a shaded surface, so it too carries no edge overlay (ADR-0044).
    assert!(batch.uvs.is_empty());
    assert_eq!(batch.upload_size_bytes(), 4 * 12 * 3 + 2 * 12);
}

#[test]
fn mesh_texture_keys_collect_the_base_colour_and_all_four_pbr_keys() {
    let delta = SceneCache::default()
        .build(&mesh_representation(1, pbr_quad()), stamp())
        .unwrap();
    let keys = mesh_texture_keys(delta.added.iter());
    let names = keys.iter().map(|key| key.as_str()).collect::<Vec<_>>();
    let mut expected = vec![TEXTURE_KEY, MR_KEY, NORMAL_KEY, EMISSIVE_KEY, OCCLUSION_KEY];
    expected.sort_unstable();
    assert_eq!(names, expected, "all five keys, deduped and sorted");
}

// ---- PBR meshes carry no triangle-derived wireframe (ADR-0044) ----------

#[test]
fn pbr_mesh_emits_no_edge_overlay() {
    // The same quad twice: textured but not PBR keeps yesterday's overlay, PBR
    // gets the `MeshWithoutEdgeOverlay` shape with identical mesh streams.
    let plain_delta = SceneCache::default()
        .build(&mesh_representation(30, textured_quad()), stamp())
        .unwrap();
    let pbr_delta = SceneCache::default()
        .build(&mesh_representation(31, pbr_quad()), stamp())
        .unwrap();
    let plain = &plain_delta.added[0];
    let shaded = &pbr_delta.added[0];

    assert!(plain.pbr.is_none(), "the control mesh has no PBR material");
    assert!(shaded.pbr.is_some(), "the PBR material survives conversion");

    // A textured non-PBR mesh keeps the surface-boundary overlay: the quad's
    // tessellation diagonal cancels, leaving 4 boundary segments.
    assert_eq!(
        plain.edges.len(),
        8,
        "a textured but non-PBR mesh keeps today's overlay"
    );
    assert!(
        shaded.edges.is_empty(),
        "an imported PBR mesh emits zero edge-overlay positions"
    );

    // Every other stream is the same as today.
    assert_eq!(shaded.topology, plain.topology);
    assert_eq!(shaded.local_origin, plain.local_origin);
    assert_eq!(shaded.vertices, plain.vertices);
    assert_eq!(shaded.normals, plain.normals);
    assert_eq!(shaded.colors, plain.colors);
    assert_eq!(shaded.uvs, plain.uvs);
    assert_eq!(shaded.indices, plain.indices);
    assert_eq!(
        shaded.texture.as_ref().map(|key| key.as_str()),
        plain.texture.as_ref().map(|key| key.as_str())
    );

    // No edges must not turn the shaded asset into the navigation-time text
    // glyph class: a PBR mesh stays visible while the camera moves.
    assert!(
        !shaded.is_text_glyph(),
        "a PBR mesh is a shaded surface, not text"
    );
}

#[test]
fn a_body3d_still_emits_its_stored_edges() {
    // ADR-0044 only removes the *triangle-derived* overlay of imported PBR
    // meshes. A `Body3d` reaches the scene as `MeshWithStoredEdges`, and its
    // stored wireframe must render unchanged — even if the display mesh ever
    // carried a PBR material.
    let mesh = pbr_quad();
    let stored = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    ];
    let mut representation = mesh_representation(32, mesh.clone());
    representation.fragments[0].primitive = DisplayPrimitive::MeshWithStoredEdges {
        mesh: Arc::new(mesh),
        edges: Arc::from(stored.into_boxed_slice()),
        derive_when_empty: true,
    };
    let delta = SceneCache::default()
        .build(&representation, stamp())
        .unwrap();
    let batch = &delta.added[0];
    assert_eq!(
        batch.edges,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        "the body's stored segment is still drawn"
    );
    assert!(batch.pbr.is_some(), "the material survives conversion");
    assert!(!batch.is_text_glyph());
}
