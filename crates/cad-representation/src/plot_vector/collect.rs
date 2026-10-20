use cad_domain::{CadResult, ObjectId, Point3};

use crate::{DisplayFragment, DisplayPrimitive, DisplayRepresentation, PlotPage};

use super::model::{VectorDocument, VectorPath};
use super::page::VectorPage;
use super::vector_diagnostic;

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

/// Collect the path geometry of a display representation onto a planned page.
///
/// Never fails for an unsupported primitive: those are recorded in the
/// document's diagnostics and completeness instead of being dropped. It only
/// fails when the page itself cannot define a physical transform.
pub fn build_vector_document(
    representation: &DisplayRepresentation,
    page: &PlotPage,
) -> CadResult<VectorDocument> {
    let vector_page = VectorPage::from_plot_page(page)?;
    let mut document = VectorDocument::empty(&vector_page);
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
                for pair in points.chunks_exact(2) {
                    collect_lines(&mut document, object, pair, fragment, &vector_page);
                }
            }
            // This slice keeps the chord draw: expand the conic through the same
            // tessellation the pre-conic path used. A normal paper-space build
            // has already expanded conics, so this is a safety net.
            DisplayPrimitive::Conic(conic) => {
                let points = conic.chord_points();
                collect_lines(&mut document, object, &points, fragment, &vector_page);
            }
            DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                collect_mesh(&mut document, object, mesh, fragment, &vector_page);
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
            // A mask has no path representation: it erases in the raster frame by
            // flooding the background. Vector output therefore skips it and names
            // the omission (ADR 0022 D4), never silently.
            DisplayPrimitive::Mask(_) => {
                document.report(
                    object,
                    vector_diagnostic::MASK_UNSUPPORTED,
                    "wipeout mask is skipped in vector output: there is no background flood in a path writer"
                        .to_string(),
                );
            }
        }
    }
    Ok(document)
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
