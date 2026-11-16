use cad_domain::{CadResult, ObjectId, Point3};

use crate::{DisplayFragment, DisplayPrimitive, DisplayRepresentation, PlotPage};

use super::model::{VectorDocument, VectorPath};
use super::page::VectorPage;
use super::vector_diagnostic;

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

/// One WIPEOUT mask waiting for the document's final mask stage.
struct DeferredMask {
    /// The source entity's display order (masks paint in ascending order).
    draw_order: i64,
    /// The boundary in page millimetres, y-up.
    boundary: Vec<[f64; 2]>,
    /// `WIPEOUTFRAME` 1 (plot the frame): emit the closed boundary outline after
    /// the fill. `2`/`0` arrive here with `false` because the plot context is
    /// built with the frame flag clear.
    frame: bool,
    /// The fragment's opacity (a mask erases, so this is the entity's alpha).
    opacity: f32,
    /// Extra even-odd rings for the fill. Empty for a normal mask; one page
    /// rectangle for an `inverted` mask, whose boundary is then a hole in it (the
    /// raster complement expressed as a multi-ring path).
    extra_rings: Vec<Vec<[f64; 2]>>,
    /// The frame's resolved colour and lineweight (the mask fill itself is the
    /// page background, never this colour).
    frame_color: [f32; 3],
    frame_lineweight: f64,
}

/// Collect the path geometry of a display representation onto a planned page.
///
/// Never fails for an unsupported primitive: those are recorded in the
/// document's diagnostics and completeness instead of being dropped. It only
/// fails when the page itself cannot define a physical transform.
///
/// WIPEOUT masks are **deferred** to a final stage: the raster renderer paints
/// every mask in a last `depth_compare: Always` pass (ADR 0022), so the path
/// document paints each mask's fill after every regular path — otherwise a
/// mask's page-background fill would be overpainted by geometry the mask is
/// supposed to hide. Masks paint in ascending `draw_order` (stable), then their
/// boundary frames, exactly like the renderer's per-mask fill-then-frame draw.
pub fn build_vector_document(
    representation: &DisplayRepresentation,
    page: &PlotPage,
) -> CadResult<VectorDocument> {
    let vector_page = VectorPage::from_plot_page(page)?;
    let mut document = VectorDocument::empty(&vector_page);
    let mut masks: Vec<DeferredMask> = Vec::new();
    for fragment in &representation.fragments {
        let object = Some(ObjectId(fragment.source.entity.0));
        match &fragment.primitive {
            DisplayPrimitive::Lines(points) => {
                collect_lines(&mut document, object, points, fragment, &vector_page);
            }
            DisplayPrimitive::LineSegments(points) => {
                if points.len() % 2 != 0 {
                    document.report(
                        object,
                        vector_diagnostic::DEGENERATE_POLYLINE,
                        "line segments require endpoint pairs".into(),
                    );
                }
                for pair in points.as_chunks::<2>().0 {
                    collect_lines(&mut document, object, pair, fragment, &vector_page);
                }
            }
            // Vector output carries polylines only: expand the (possibly
            // viewport-clipped) analytic conic through the same chord
            // tessellation the pre-conic path used.
            DisplayPrimitive::Conic(conic) => {
                let points = conic.chord_points();
                collect_lines(&mut document, object, &points, fragment, &vector_page);
            }
            DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                collect_mesh(&mut document, object, mesh, fragment, &vector_page);
            }
            // A body is drawn as its filled mesh **plus** the stored wireframe,
            // so the path document carries the same two layers the raster does.
            // The edge list is a flat set of independent endpoint pairs (the
            // `LineSegments` pairing rule), never a polyline.
            DisplayPrimitive::MeshWithStoredEdges { mesh, edges } => {
                collect_mesh(&mut document, object, mesh, fragment, &vector_page);
                if edges.len() % 2 != 0 {
                    document.report(
                        object,
                        vector_diagnostic::DEGENERATE_POLYLINE,
                        "stored body edges require endpoint pairs".into(),
                    );
                }
                for pair in edges.as_chunks::<2>().0 {
                    collect_lines(&mut document, object, pair, fragment, &vector_page);
                }
            }
            DisplayPrimitive::Text { .. } => {
                document.report(
                    object,
                    vector_diagnostic::TEXT_UNSHAPED,
                    "text was not shaped into outlines; vector output cannot carry an unshaped font"
                        .to_string(),
                );
            }
            DisplayPrimitive::Image { .. } => {
                document.report(
                    object,
                    vector_diagnostic::IMAGE_UNSUPPORTED,
                    "image primitives have no path representation in this writer".to_string(),
                );
            }
            DisplayPrimitive::Instance { .. } => {
                document.report(
                    object,
                    vector_diagnostic::UNEXPANDED_INSTANCE,
                    "INSERT instance reached the vector stage unexpanded; use build_expanded/build_paper_space"
                        .to_string(),
                );
            }
            // A mask **does** have a path representation: its fill is the page
            // background colour, not a transparent hole, so the paths painted
            // before it really disappear — the paper equivalent of the raster
            // clear-colour flood (ADR 0022 D1). The fill is deferred to the
            // document's final stage because the raster mask pass paints last
            // (ADR 0022 D2 subset). The boundary **frame** is real vector
            // geometry: when the mask carries `frame` it is emitted as a closed
            // polyline after its own fill (`WIPEOUTFRAME` 1 plots the frame; 2
            // does not, because the plot context is built without it).
            //
            // `inverted` (DXF clip mode Inside) masks everything *outside* its
            // boundary. The raster renderer paints exactly that by prepending a
            // covering ring to the boundary and filling both even-odd
            // (`inverted_mask_fill` in `cad-render-wgpu`), so the path document
            // does the same with the page rectangle as the covering ring: an area
            // inside the boundary is covered twice and stays clear, every other
            // page point is covered once and is flooded. A vector page has no
            // camera, so the ring is the fixed page (the raster's ring is the
            // visible rect) — for a plot the visible rect *is* the page.
            DisplayPrimitive::Mask {
                boundary,
                inverted,
                frame,
            } => {
                if boundary.len() < 3 {
                    document.report(
                        object,
                        vector_diagnostic::MASK_BOUNDARY_DEGENERATE,
                        "wipeout mask boundary has fewer than three vertices; the mask fill is \
                         not drawn"
                            .to_string(),
                    );
                } else {
                    let Some(boundary) = map_points(boundary, &vector_page) else {
                        document.report(
                            object,
                            vector_diagnostic::NON_FINITE,
                            "wipeout mask boundary has a non-finite mapped coordinate and was \
                             omitted"
                                .to_string(),
                        );
                        continue;
                    };
                    masks.push(DeferredMask {
                        draw_order: fragment.draw_order,
                        boundary,
                        frame: *frame,
                        opacity: fragment.alpha,
                        extra_rings: if *inverted {
                            vec![page_ring(&vector_page)]
                        } else {
                            Vec::new()
                        },
                        frame_color: fragment.color,
                        frame_lineweight: finite_lineweight(fragment.lineweight),
                    });
                }
            }
        }
    }
    // The final mask stage: fills (ascending display order) then each mask's
    // boundary frame, mirroring the renderer's per-mask draw.
    masks.sort_by_key(|mask| mask.draw_order);
    let background = document.background;
    for mask in &masks {
        if mask.extra_rings.is_empty() {
            document.paths.push(VectorPath::polygon(
                mask.boundary.clone(),
                background,
                mask.opacity,
            ));
        } else {
            // The boundary first, then the covering page ring: under even-odd the
            // union order is irrelevant, and keeping the entity's own ring first
            // makes the path's `points` the mask the user drew.
            let mut rings = Vec::with_capacity(1 + mask.extra_rings.len());
            rings.push(mask.boundary.clone());
            rings.extend(mask.extra_rings.iter().cloned());
            document
                .paths
                .push(VectorPath::even_odd_fill(rings, background, mask.opacity));
        }
        if mask.frame {
            push_mask_frame_mapped(
                &mut document,
                &mask.boundary,
                mask.frame_color,
                mask.frame_lineweight,
            );
        }
    }
    Ok(document)
}

/// The document page as one closed ring, in page millimetres (y-up).
///
/// It is the vector counterpart of the raster "covering ring": the even-odd
/// partner of an inverted mask boundary.
fn page_ring(page: &VectorPage) -> Vec<[f64; 2]> {
    vec![
        [0.0, 0.0],
        [page.width_mm, 0.0],
        [page.width_mm, page.height_mm],
        [0.0, page.height_mm],
    ]
}

/// Emit one mask's closed boundary frame from page-space points.
///
/// The ring repeats its first point so a stroked open polyline closes visually;
/// the frame is stroked, never filled.
fn push_mask_frame_mapped(
    document: &mut VectorDocument,
    boundary: &[[f64; 2]],
    color: [f32; 3],
    lineweight: f64,
) {
    let mut outline = boundary.to_vec();
    outline.push(boundary[0]);
    document
        .paths
        .push(VectorPath::polyline(outline, color, lineweight, 1.0));
}

fn collect_lines(
    document: &mut VectorDocument,
    object: Option<ObjectId>,
    points: &[Point3],
    fragment: &DisplayFragment,
    page: &VectorPage,
) {
    if points.len() < 2 {
        document.report(
            object,
            vector_diagnostic::DEGENERATE_POLYLINE,
            format!(
                "polyline has {} point(s); at least two are needed to draw",
                points.len()
            ),
        );
        return;
    }
    let Some(mapped) = map_points(points, page) else {
        document.report(
            object,
            vector_diagnostic::NON_FINITE,
            "polyline has a non-finite mapped coordinate and was omitted".to_string(),
        );
        return;
    };
    let width = finite_lineweight(fragment.lineweight);
    document.paths.push(VectorPath::polyline(
        mapped,
        fragment.color,
        width,
        fragment.alpha,
    ));
}

fn collect_mesh(
    document: &mut VectorDocument,
    object: Option<ObjectId>,
    mesh: &cad_domain::Mesh,
    fragment: &DisplayFragment,
    page: &VectorPage,
) {
    if mesh.triangles.is_empty() {
        document.report(
            object,
            vector_diagnostic::MESH_WITHOUT_TRIANGLES,
            "mesh has no triangles; it cannot be expressed as paths".to_string(),
        );
        return;
    }
    if !mesh.colors.is_empty() {
        document.report(
            object,
            vector_diagnostic::MESH_VERTEX_COLORS_IGNORED,
            "mesh per-vertex colours are not carried by the path model; the fragment colour is used"
                .to_string(),
        );
    }
    for triangle in &mesh.triangles {
        let mut mapped = Vec::with_capacity(3);
        let mut ok = true;
        for index in triangle {
            let Some(vertex) = mesh.vertices.get(*index as usize) else {
                ok = false;
                break;
            };
            let point = page.map_paper_point(*vertex);
            if !point[0].is_finite() || !point[1].is_finite() {
                ok = false;
                break;
            }
            mapped.push(point);
        }
        if !ok {
            document.report(
                object,
                vector_diagnostic::MESH_BAD_INDEX,
                "mesh triangle references a missing or non-finite vertex and was omitted"
                    .to_string(),
            );
            continue;
        }
        document
            .paths
            .push(VectorPath::polygon(mapped, fragment.color, fragment.alpha));
    }
}

/// Map a polyline, returning `None` if any coordinate is non-finite.
fn map_points(points: &[Point3], page: &VectorPage) -> Option<Vec<[f64; 2]>> {
    let mut out = Vec::with_capacity(points.len());
    for point in points {
        let mapped = page.map_paper_point(*point);
        if !mapped[0].is_finite() || !mapped[1].is_finite() {
            return None;
        }
        out.push(mapped);
    }
    Some(out)
}

/// A finite, non-negative lineweight in millimetres; an unusable value is 0
/// (a hairline), never a fabricated default.
fn finite_lineweight(lineweight: f32) -> f64 {
    let value = lineweight as f64;
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}
