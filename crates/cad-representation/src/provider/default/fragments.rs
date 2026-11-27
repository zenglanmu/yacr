//! The chord/tessellation half of the default provider.
//!
//! Split out of `default.rs` for the file-size ceiling (`check-file-size.py`).
//! Everything here is the *derived display* of one entity's geometry: the chord
//! budget ([`chord_tessellation_params`]), the analytic conic a curve maps to
//! ([`conic_from_geometry`]) and the two fragment builders every fallback path
//! ends in. `DefaultRepresentationProvider` stays the only place that decides
//! *which* path an entity takes.

use crate::*;

/// The chord-tessellation tolerance a build uses for curve geometry.
///
/// Shared by the provider's chord fallback and by the conic chord expansion so
/// the two can never drift apart. `VIEWRES`
/// ([`RepresentationContext::curve_segments`]) is applied here as a **ceiling**
/// on segments per curve, so every chord path — splines, polyline bulge arcs,
/// the conic chord fallback and the wide-polyline band — reads one budget.
///
/// Two honest facts travel with that:
///
/// * the tolerance is still the display-pixel budget, so raising `VIEWRES`
///   above what the tolerance already asks for changes nothing (it is a
///   ceiling, not a smoothness setting);
/// * circles, arcs and ellipses reach the display as analytic conics and are
///   never chord-tessellated there, so `VIEWRES` cannot make them smoother or
///   coarser. Only the paths that really produce chords are bounded.
pub(crate) fn chord_tessellation_params(context: &RepresentationContext) -> TessellationParams {
    let mut params = TessellationParams {
        tolerance: context.tolerance.display_pixels.max(1e-12),
        ..TessellationParams::default()
    };
    if let Some(ceiling) = context.curve_segments {
        // `arc_segments_for_tolerance` clamps with `min.max(1)..max.max(1)`, so
        // the floor must never exceed the ceiling: a `VIEWRES` ceiling below the
        // default floor lowers the floor with it instead of panicking.
        params.max_segments = ceiling.max(1);
        params.min_segments = params.min_segments.min(params.max_segments);
    }
    params
}

/// Build the analytic conic for a curve entity, or `None` when the source is
/// degenerate or non-finite (the caller then keeps the legacy chord path).
///
/// The semi-diameter vectors mirror the frame [`tessellate_geometry`] uses, so
/// expanding the conic back through that same tessellation reproduces the
/// pre-conic chord output.
pub(super) fn conic_from_geometry(geometry: &SemanticGeometry) -> Option<ConicSegment> {
    match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let conic = ConicSegment {
                center: *center,
                major: cad_geometry::scale(ax, *radius),
                minor: cad_geometry::scale(ay, *radius),
                start: 0.0,
                sweep: std::f64::consts::TAU,
                full: true,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let conic = ConicSegment {
                center: *center,
                major: cad_geometry::scale(ax, *radius),
                minor: cad_geometry::scale(ay, *radius),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= std::f64::consts::TAU - 1e-9,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = cad_geometry::length(*major_axis);
            if !(major_len.is_finite() && major_len > 0.0) {
                return None;
            }
            let u = cad_geometry::scale(*major_axis, 1.0 / major_len);
            // The minor direction is the in-plane +90° rotation of the major
            // axis, matching `ellipse_minor_dir` in the tessellation frame.
            let minor_dir = {
                let c = cad_geometry::cross(*normal, u);
                let len = cad_geometry::length(c);
                if len >= 1e-12 {
                    cad_geometry::scale(c, 1.0 / len)
                } else {
                    return None;
                }
            };
            let conic = ConicSegment {
                center: *center,
                major: *major_axis,
                minor: cad_geometry::scale(minor_dir, major_len * ratio.abs()),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= std::f64::consts::TAU - 1e-9,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        _ => None,
    }
}

/// Push one prepared triangle mesh (for example a wide polyline stroke band)
/// with the same pre-expansion style placeholders every provider fragment
/// carries.
///
/// The placeholders are resolved by `expand_entity` (ByLayer/ByBlock and layer
/// style), so the stroke follows the entity's resolved colour, lineweight,
/// linetype and alpha exactly like the centreline path. The mesh reaches the
/// clip stage as [`DisplayPrimitive::Mesh`], which `emit_clipped` cuts with
/// `clip_mesh_to_rect`, so an `XCLIP` rectangle clips the stroke too.
pub(super) fn push_mesh_fragment(
    entity: &DbEntity,
    mesh: Mesh,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    if mesh.triangles.is_empty() {
        representation.completeness = weaker_completeness(
            &representation.completeness,
            &Completeness::Partial(vec!["the wide polyline stroke produced no triangles".into()]),
        );
        representation.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "representation.polyline_stroke_empty".into(),
            message: "the wide polyline stroke produced no triangles".into(),
        });
        return;
    }
    representation.fragments.push(DisplayFragment {
        source,
        geometry_source,
        precision: Precision::Analytic,
        alpha: 1.0,
        draw_order: entity.draw_order,
        color: DEFAULT_RENDER_COLOR,
        color_unresolved: true,
        lineweight: DEFAULT_LINEWEIGHT_MM,
        lineweight_unresolved: true,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: true,
        linetype_scale: 1.0,
        primitive: DisplayPrimitive::Mesh(Arc::new(mesh)),
    });
}

/// Emit a [`SemanticGeometry::Body3d`]'s derived display cache with the existing
/// primitives.
///
/// A body reaches the display through the cache `cad-db` stores beside its exact
/// geometry, never by re-tessellating the B-rep here: a filled mesh when the
/// cache has triangles, otherwise its boundary/feature edges as independent
/// segments (the `BodyKind::Wire` / edge-only case). Rebuilding geometry at draw
/// time would be a second source of truth beside the cache.
///
/// Precision and completeness are the body's own and are never assumed or
/// upgraded: an approximate cache stays approximate, and a `Partial`/`Missing`
/// display verdict is carried through unchanged. The geometry source separates a
/// body whose exact B-rep is present ([`GeometrySource::KernelMesh`]: the cache
/// was derived from the kernel) from a display-only body
/// ([`GeometrySource::DirectMesh`]).
///
/// A body with neither a mesh nor drawable edges is missing geometry with an
/// explicit diagnostic — never an empty success.
///
/// When the cache has both triangles and stored edges, the fragment is a
/// [`DisplayPrimitive::MeshWithStoredEdges`]: the stored edge set is the body's
/// wireframe source, so the scene fills `RenderBatch::edges` from it instead of
/// re-deriving an overlay from the triangle topology. A cache with no stored
/// edges keeps the plain [`DisplayPrimitive::Mesh`] (legacy triangle-derived
/// overlay, and the text-LOD classification is unchanged).
///
/// # `VSCURRENT WIREFRAME`
///
/// [`RepresentationContext::body_wireframe`] switches the **body only** to a
/// 2D-wireframe display: the fragment becomes
/// [`DisplayPrimitive::LineSegments`] instead of a mesh.
///
/// * stored body edges are used as they are (the same flattened endpoint pairs
///   the shaded variant carries);
/// * a triangulated body with **no** stored edges (a sphere/torus full primitive
///   has no boundary edges in its cache) gets the unique triangle-edge segments
///   derived from the display mesh, de-duplicated by unordered endpoint pair in
///   first-occurrence order — deterministic, and never an empty fragment that
///   would make the body invisible;
/// * a wire body (no mesh) keeps its [`DisplayPrimitive::LineSegments`], exactly
///   as when shaded;
/// * a body with neither a mesh nor edges stays the explicit `Missing` verdict.
///
/// The flag changes nothing for other entities: there is no hidden-line removal
/// and no DB write.
pub(super) fn body_fragment(
    entity: &DbEntity,
    body: &Body3d,
    context: &RepresentationContext,
    source: SelectionRef,
    representation: &mut DisplayRepresentation,
    soften: Option<f64>,
) {
    let soften = soften.filter(|angle| angle.is_finite() && (0.0..=180.0).contains(angle));
    let geometry_source = if body.is_exact() {
        GeometrySource::KernelMesh
    } else {
        GeometrySource::DirectMesh
    };
    let edge_color = context.edge_color.map(|rgb| {
        [
            rgb[0] as f32 / 255.0,
            rgb[1] as f32 / 255.0,
            rgb[2] as f32 / 255.0,
        ]
    });
    let fragment = |primitive: DisplayPrimitive, color: [f32; 3]| DisplayFragment {
        source: source.clone(),
        geometry_source: geometry_source.clone(),
        precision: body.display.precision.clone(),
        alpha: 1.0,
        draw_order: entity.draw_order,
        color,
        color_unresolved: color == DEFAULT_RENDER_COLOR,
        lineweight: DEFAULT_LINEWEIGHT_MM,
        lineweight_unresolved: true,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: true,
        linetype_scale: 1.0,
        primitive,
    };
    // `VSCURRENT WIREFRAME`: edges only, from the stored body edges when the
    // cache has them, otherwise from the display mesh's unique triangle edges.
    if context.body_wireframe {
        if !context.body_edges {
            // Wireframe with Edges off has no fill and no edges: an explicit
            // style result, not missing geometry.
            representation.completeness = body.display.completeness.clone();
            return;
        }
        let stored = body_edge_segments(&body.display.edges);
        let segments = if stored.is_empty() {
            triangle_edge_segments(&body.display.mesh)
        } else {
            stored
        };
        let segments = match soften {
            Some(angle) => soften_edge_segments(&body.display.mesh, &segments, angle),
            None => segments,
        };
        if !segments.is_empty() {
            representation.fragments.push(fragment(
                DisplayPrimitive::LineSegments(Arc::from(segments.into_boxed_slice())),
                edge_color.unwrap_or(DEFAULT_RENDER_COLOR),
            ));
            representation.completeness = body.display.completeness.clone();
            return;
        }
        // Every stored edge is softened away: a wireframe request with no hard
        // edge left draws nothing (no filled fallback), and the empty result is
        // the soft state, not a missing geometry.
        if soften.is_some() {
            representation.completeness = body.display.completeness.clone();
            return;
        }
        // A triangulated cache whose every edge is degenerate still must not
        // fall back to a filled mesh: the wireframe request was explicit.
        if !body.display.mesh.triangles.is_empty() {
            representation.completeness =
                body.display
                    .completeness
                    .clone()
                    .combine(Completeness::Missing(vec![format!(
                        "the {:?} body's display mesh produced no drawable wireframe segment",
                        body.kind
                    )]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "representation.body_wireframe_empty".into(),
                message: format!(
                    "{} is a {:?} body whose display mesh has no drawable triangle edge; \
                     WIREFRAME draws nothing rather than a filled fallback",
                    entity.object.type_key, body.kind
                ),
            });
            return;
        }
    }
    // A cache with no triangles draws nothing as a filled mesh, and the scene
    // reads an index-less `RenderTopology::Mesh` batch as `vertices.len() / 3`
    // triangles, which would fabricate topology. Only a triangulated cache takes
    // the mesh path; a vertices-only cache falls through to its edges.
    if !body.display.mesh.triangles.is_empty() {
        // Flatten the stored edge polylines to independent endpoint pairs. They
        // travel with the mesh only when at least one segment survives; an empty
        // edge set keeps today's plain `Mesh`, whose overlay is derived from the
        // triangles and whose display class stays "not a text glyph".
        let stored = body_edge_segments(&body.display.edges);
        // A coloured edge overlay is a separate line fragment: the mesh keeps
        // its fill, the stored edges take the session colour.
        if context.body_edges && !stored.is_empty() {
            if let Some(color) = edge_color {
                let edges = match soften {
                    Some(angle) => soften_edge_segments(&body.display.mesh, &stored, angle),
                    None => stored,
                };
                representation.fragments.push(fragment(
                    DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(body.display.mesh.clone())),
                    DEFAULT_RENDER_COLOR,
                ));
                if !edges.is_empty() {
                    representation.fragments.push(fragment(
                        DisplayPrimitive::LineSegments(Arc::from(edges.into_boxed_slice())),
                        color,
                    ));
                }
                representation.completeness = body.display.completeness.clone();
                return;
            }
        }
        let primitive = if !context.body_edges {
            // Edges off: keep the fill but never emit a stored or derived edge.
            DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(body.display.mesh.clone()))
        } else if stored.is_empty() {
            // No stored edge exists to soften: the boundary overlay is the
            // body's own (odd-use) edges, so a `Mesh` is already edge-exact.
            DisplayPrimitive::Mesh(Arc::new(body.display.mesh.clone()))
        } else {
            let edges = match soften {
                Some(angle) => soften_edge_segments(&body.display.mesh, &stored, angle),
                None => stored,
            };
            DisplayPrimitive::MeshWithStoredEdges {
                mesh: Arc::new(body.display.mesh.clone()),
                edges: Arc::from(edges.into_boxed_slice()),
                // A softened body keeps its fill with no hard edge when every
                // stored edge is filtered out; the legacy overlay fallback must
                // not reappear.
                derive_when_empty: soften.is_none(),
            }
        };
        representation
            .fragments
            .push(fragment(primitive, DEFAULT_RENDER_COLOR));
        representation.completeness = body.display.completeness.clone();
        return;
    }
    let segments = body_edge_segments(&body.display.edges);
    if !context.body_edges {
        representation.completeness = body.display.completeness.clone();
        return;
    }
    if !segments.is_empty() {
        representation.fragments.push(fragment(
            DisplayPrimitive::LineSegments(Arc::from(segments.into_boxed_slice())),
            edge_color.unwrap_or(DEFAULT_RENDER_COLOR),
        ));
        representation.completeness = body.display.completeness.clone();
        return;
    }
    // No mesh and no drawable edges: the cache carries no display geometry at
    // all, so this is the crate's missing-geometry verdict, never an empty
    // success. `combine` keeps the body's own reasons and appends this one.
    representation.completeness = body
        .display
        .completeness
        .clone()
        .combine(Completeness::Missing(vec![format!(
            "the {:?} body has neither a display mesh nor edge polylines",
            body.kind
        )]));
    representation.diagnostics.push(Diagnostic {
        object: Some(ObjectId(entity.id.0)),
        code: "representation.body_empty".into(),
        message: format!(
            "{} is a {:?} body with no display mesh and no drawable edge polylines",
            entity.object.type_key, body.kind
        ),
    });
}

/// Flatten a body's edge polylines into independent endpoint pairs.
///
/// Each polyline becomes its consecutive segments, and a polyline with fewer
/// than two points has no segment and is dropped — the provider never invents a
/// vertex. The pairs are unordered, which is exactly what
/// [`DisplayPrimitive::LineSegments`] draws.
fn body_edge_segments(edges: &[Vec<Point3>]) -> Vec<Point3> {
    let mut segments = Vec::new();
    for polyline in edges {
        for pair in polyline.windows(2) {
            segments.push(pair[0]);
            segments.push(pair[1]);
        }
    }
    segments
}

/// Derive the **unique** triangle-edge segments of a display mesh.
///
/// Used only by `VSCURRENT WIREFRAME` for a triangulated body whose cache has no
/// stored boundary edges (sphere/torus full primitives). Every triangle
/// contributes its three edges; a shared edge (a quad diagonal, a sphere mesh
/// seam) appears once, keyed by its unordered endpoint pair in first-occurrence
/// order, so the output is deterministic for a given mesh. Non-finite endpoints
/// and zero-length pairs are dropped rather than forwarded to the renderer. A
/// triangle index outside the vertex array is skipped — the provider never reads
/// past the mesh it was handed.
fn triangle_edge_segments(mesh: &Mesh) -> Vec<Point3> {
    use std::collections::HashSet;

    fn key(point: Point3) -> Option<[u64; 3]> {
        let coords = [point.x, point.y, point.z];
        if coords.iter().any(|value| !value.is_finite()) {
            return None;
        }
        Some([point.x.to_bits(), point.y.to_bits(), point.z.to_bits()])
    }

    let mut seen: HashSet<[[u64; 3]; 2]> = HashSet::new();
    let mut segments = Vec::new();
    fn push(a: Point3, b: Point3, seen: &mut HashSet<[[u64; 3]; 2]>, segments: &mut Vec<Point3>) {
        let (Some(a_key), Some(b_key)) = (key(a), key(b)) else {
            return;
        };
        if a_key == b_key {
            return;
        }
        let pair = if a_key <= b_key {
            [a_key, b_key]
        } else {
            [b_key, a_key]
        };
        if seen.insert(pair) {
            segments.push(a);
            segments.push(b);
        }
    }
    for triangle in &mesh.triangles {
        let [i, j, k] = *triangle;
        let (Some(a), Some(b), Some(c)) = (
            mesh.vertices.get(i as usize),
            mesh.vertices.get(j as usize),
            mesh.vertices.get(k as usize),
        ) else {
            continue;
        };
        push(*a, *b, &mut seen, &mut segments);
        push(*b, *c, &mut seen, &mut segments);
        push(*c, *a, &mut seen, &mut segments);
    }
    segments
}

/// Tessellate a curve-like entity into a chord polyline and push it as a
/// `Lines` fragment (the pre-conic display path).
///
/// A shape that yields fewer than two points becomes an explicit `Missing`
/// result with a diagnostic, never an empty success.
pub(super) fn push_chord_fragment(
    entity: &DbEntity,
    geometry: &SemanticGeometry,
    context: &RepresentationContext,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    let points = tessellate_geometry(geometry, chord_tessellation_params(context));
    if points.len() >= 2 {
        representation.fragments.push(DisplayFragment {
            source,
            geometry_source,
            precision: Precision::Analytic,
            alpha: 1.0,
            draw_order: entity.draw_order,
            color: DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
        });
    } else {
        representation.completeness =
            Completeness::Missing(vec!["no display representation in this build".into()]);
        representation.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "representation.empty".into(),
            message: format!("{} produced no drawable points", entity.object.type_key),
        });
    }
}

/// The list of display edges whose adjacent triangles meet **above** `angle`
/// degrees; an edge between two faces at or below the threshold is softened away.
///
/// Only edges that coincide exactly with a display-triangle edge are candidates
/// (the stored edge endpoints are the mesh's own vertices for an imported body).
/// A boundary edge (one adjacent triangle), a non-manifold edge (three or more)
/// or an unmatched segment is kept: softening never invents a hidden silhouette,
/// and an edge the mesh cannot explain stays visible.
fn soften_edge_segments(mesh: &cad_domain::Mesh, segments: &[Point3], angle: f64) -> Vec<Point3> {
    use std::collections::HashMap;
    type Key = ((u64, u64, u64), (u64, u64, u64));
    let vertex_key = |point: Point3| {
        let bits = |value: f64| if value == 0.0 { 0 } else { value.to_bits() };
        (bits(point.x), bits(point.y), bits(point.z))
    };
    let edge_key = |a: Point3, b: Point3| {
        let (a, b) = (vertex_key(a), vertex_key(b));
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    };
    let mut normals: HashMap<Key, Vec<[f64; 3]>> = HashMap::new();
    for triangle in &mesh.triangles {
        let indices = triangle.map(|index| index as usize);
        if indices.iter().any(|index| *index >= mesh.vertices.len()) {
            continue;
        }
        let [a, b, c] = indices.map(|index| mesh.vertices[index]);
        let u = [b.x - a.x, b.y - a.y, b.z - a.z];
        let v = [c.x - a.x, c.y - a.y, c.z - a.z];
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = normal[0].hypot(normal[1]).hypot(normal[2]);
        if !length.is_finite() || length <= 1e-12 {
            continue;
        }
        let normal = [normal[0] / length, normal[1] / length, normal[2] / length];
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let key = edge_key(mesh.vertices[indices[i]], mesh.vertices[indices[j]]);
            normals.entry(key).or_default().push(normal);
        }
    }
    segments
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| {
            let Some(adjacent) = normals.get(&edge_key(pair[0], pair[1])) else {
                return true;
            };
            let [first, second] = adjacent.as_slice() else {
                return true;
            };
            let dot = (first[0] * second[0] + first[1] * second[1] + first[2] * second[2])
                .clamp(-1.0, 1.0);
            dot.acos().to_degrees() > angle
        })
        .flatten()
        .copied()
        .collect()
}
