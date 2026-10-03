//! Vector plot export: paper-space display geometry → path document → SVG/PDF.
//!
//! This module is the vector counterpart of the raster `plot` path. It takes the
//! same CPU [`DisplayRepresentation`] the renderer builds (for a paper-space
//! layout, that is [`build_paper_space`], which has already applied every
//! viewport's model→paper transform and expanded INSERTs) plus the [`PlotPage`]
//! the pure planner produced, and turns it into a small, self-contained vector
//! document that can be serialised as SVG or a minimal single-page PDF.
//!
//! ## No GPU
//!
//! Every function here is pure CPU: no wgpu device, no window, no surface. The
//! SVG/PDF writers are hand-written string/byte builders (no third-party
//! crates), so a caller can exercise the whole path in a unit test without a
//! software Vulkan adapter.
//!
//! ## Coordinate system
//!
//! The document coordinate system is **page millimetres, origin at the
//! bottom-left, y pointing up** — the CAD paper convention. [`VectorPage`]
//! derives it from a [`PlotPage`] by undoing the planner's pixel scale and y
//! flip, so the physical size is DPI-independent up to the plan's own pixel
//! rounding. SVG uses y-down user units, so the SVG writer flips y about the
//! document height; PDF user space is already y-up, so the PDF writer emits
//! millimetres directly behind a `72/25.4` scale matrix.
//!
//! ## Honesty
//!
//! A primitive that cannot be expressed as paths — an unshaped
//! [`DisplayPrimitive::Text`], an [`DisplayPrimitive::Image`], an unexpanded
//! [`DisplayPrimitive::Instance`], or a mesh with no triangles — is **never
//! silently dropped**. Each case adds a stable diagnostic and downgrades the
//! document's [`Completeness`] so the caller can report the gap. A degenerate
//! or non-finite polyline is reported the same way.

use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult, Completeness, Diagnostic, LayoutId, ObjectId, Point3};

use super::{
    build_paper_space, plan_plot_for_record, DisplayFragment, DisplayPrimitive,
    DisplayRepresentation, PlotPage, PlotTarget, ProviderRegistry, RepresentationContext,
    MM_PER_INCH,
};

/// Stable diagnostic codes attached to [`VectorDocument::diagnostics`].
///
/// They are part of the contract: callers may branch on the code and the human
/// message may change without breaking them.
pub mod vector_diagnostic {
    /// A polyline had fewer than two usable points.
    pub const DEGENERATE_POLYLINE: &str = "vector.degenerate_polyline";
    /// A mapped coordinate was not finite and could not be written.
    pub const NON_FINITE: &str = "vector.non_finite";
    /// A mesh carried no triangles.
    pub const MESH_WITHOUT_TRIANGLES: &str = "vector.mesh_without_triangles";
    /// A mesh triangle referenced a vertex that does not exist.
    pub const MESH_BAD_INDEX: &str = "vector.mesh_bad_index";
    /// A mesh had per-vertex colours the path model cannot carry.
    pub const MESH_VERTEX_COLORS_IGNORED: &str = "vector.mesh_vertex_colors_ignored";
    /// A text primitive was not shaped into outlines (no font available).
    pub const TEXT_UNSHAPED: &str = "vector.text_unshaped";
    /// An image primitive cannot be represented as paths here.
    pub const IMAGE_UNSUPPORTED: &str = "vector.image_unsupported";
    /// An INSERT instance reached the vector stage unexpanded.
    pub const UNEXPANDED_INSTANCE: &str = "vector.unexpanded_instance";
}

// ---------------------------------------------------------------------------
// Page transform
// ---------------------------------------------------------------------------

/// Maps paper millimetres onto document millimetres (y-up, origin bottom-left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorPage {
    /// Document width in millimetres.
    pub width_mm: f64,
    /// Document height in millimetres.
    pub height_mm: f64,
    /// Affine paper (mm) → document (mm) as `[a, b, c, d, e, f]`:
    /// `x = a*px + c*py + e`, `y = b*px + d*py + f`.
    pub paper_to_document: [f64; 6],
}

impl VectorPage {
    /// Derive the y-up document transform from a planned pixel page.
    ///
    /// [`PlotPage::paper_to_pixel`] is y-down and scaled by `pixels_per_mm`;
    /// dividing by that scale and flipping y about the document height yields
    /// the physical, y-up page. The physical size is therefore
    /// `width / pixels_per_mm` and `height / pixels_per_mm`.
    pub fn from_plot_page(page: &PlotPage) -> CadResult<VectorPage> {
        let ppm = page.pixels_per_mm;
        if !ppm.is_finite() || ppm <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "plot page has no positive pixels-per-mm scale ({ppm})"
            )));
        }
        let width_mm = page.width as f64 / ppm;
        let height_mm = page.height as f64 / ppm;
        if !width_mm.is_finite() || !height_mm.is_finite() || width_mm <= 0.0 || height_mm <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "plot page has no positive physical size ({width_mm}x{height_mm} mm)"
            )));
        }
        let m = page.paper_to_pixel;
        let paper_to_document = [
            m[0] / ppm,
            -m[1] / ppm,
            m[2] / ppm,
            -m[3] / ppm,
            m[4] / ppm,
            height_mm - m[5] / ppm,
        ];
        if !paper_to_document.iter().all(|v| v.is_finite()) {
            return Err(CadError::InvalidInput(
                "plot page transform has non-finite coefficients".into(),
            ));
        }
        Ok(VectorPage {
            width_mm,
            height_mm,
            paper_to_document,
        })
    }

    /// Map one paper-space point (millimetres) onto the document.
    pub fn map_paper_point(&self, point: Point3) -> [f64; 2] {
        let m = self.paper_to_document;
        [
            m[0] * point.x + m[2] * point.y + m[4],
            m[1] * point.x + m[3] * point.y + m[5],
        ]
    }
}

// ---------------------------------------------------------------------------
// Document model
// ---------------------------------------------------------------------------

/// One path in the document, in page-millimetre y-up coordinates.
///
/// `fill = Some` paints the interior; `stroke = Some` draws the outline with
/// `stroke_width_mm`. `opacity` is the resolved per-entity opacity in `[0, 1]`.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorPath {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
    pub stroke_width_mm: f64,
    pub opacity: f32,
}

impl VectorPath {
    /// An open stroked polyline.
    pub fn polyline(points: Vec<[f64; 2]>, stroke: [f32; 3], width_mm: f64, opacity: f32) -> Self {
        VectorPath {
            points,
            closed: false,
            fill: None,
            stroke: Some(stroke),
            stroke_width_mm: width_mm,
            opacity,
        }
    }

    /// A closed filled polygon.
    pub fn polygon(points: Vec<[f64; 2]>, fill: [f32; 3], opacity: f32) -> Self {
        VectorPath {
            points,
            closed: true,
            fill: Some(fill),
            stroke: None,
            stroke_width_mm: 0.0,
            opacity,
        }
    }
}

/// A self-contained vector page: its physical size, its path geometry and the
/// completeness verdict for geometry that could not be expressed as paths.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorDocument {
    pub width_mm: f64,
    pub height_mm: f64,
    pub paths: Vec<VectorPath>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
}

impl VectorDocument {
    /// An empty document for a page (valid SVG/PDF, no paths).
    pub fn empty(page: &VectorPage) -> Self {
        VectorDocument {
            width_mm: page.width_mm,
            height_mm: page.height_mm,
            paths: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    fn report(&mut self, object: Option<ObjectId>, code: &str, message: String) {
        self.completeness = self
            .completeness
            .clone()
            .combine(Completeness::Partial(vec![message.clone()]));
        self.diagnostics.push(Diagnostic {
            object,
            code: code.to_string(),
            message,
        });
    }
}

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
            DisplayPrimitive::Mesh(mesh) => {
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

// ---------------------------------------------------------------------------
// SVG writer
// ---------------------------------------------------------------------------

/// Serialise a document as a self-contained SVG document (UTF-8 bytes).
///
/// Width/height are declared in millimetres and the `viewBox` uses the same
/// millimetre units, so `stroke-width` is in millimetres too. No external
/// resources, fonts or scripts are referenced.
pub fn render_svg(document: &VectorDocument) -> CadResult<Vec<u8>> {
    validate_document(document)?;
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{}mm\" height=\"{}mm\" viewBox=\"0 0 {} {}\">\n",
        number(document.width_mm),
        number(document.height_mm),
        number(document.width_mm),
        number(document.height_mm),
    ));
    for path in &document.paths {
        out.push_str(&svg_path(path, document.height_mm));
    }
    out.push_str("</svg>\n");
    Ok(out.into_bytes())
}

fn svg_path(path: &VectorPath, height_mm: f64) -> String {
    let points: Vec<String> = path
        .points
        .iter()
        .map(|p| format!("{},{}", number(p[0]), number(height_mm - p[1])))
        .collect();
    let element = if path.closed { "polygon" } else { "polyline" };
    let fill = match (path.closed, path.fill) {
        (true, Some(color)) => hex_color(color),
        _ => "none".to_string(),
    };
    let mut attrs = format!("points=\"{}\" fill=\"{fill}\"", points.join(" "));
    match path.stroke {
        Some(color) => attrs.push_str(&format!(
            " stroke=\"{}\" stroke-width=\"{}\"",
            hex_color(color),
            number(path.stroke_width_mm)
        )),
        None => attrs.push_str(" stroke=\"none\""),
    }
    if path.opacity < 1.0 {
        let opacity = number(path.opacity as f64);
        attrs.push_str(&format!(
            " fill-opacity=\"{opacity}\" stroke-opacity=\"{opacity}\""
        ));
    }
    format!("  <{element} {attrs}/>\n")
}

// ---------------------------------------------------------------------------
// PDF writer
// ---------------------------------------------------------------------------

/// Serialise a document as a minimal single-page PDF (bytes).
///
/// Paths are stroked/filled at their resolved colour; the page is scaled so
/// user units are millimetres. There is no font or image resource and no
/// plot-style handling. Per-path opacity below 1 is applied through a PDF
/// ExtGState `/ca` (fill) and `/CA` (stroke) constant alpha, emitted before each
/// path with `gs` and reset to opaque again, so no other content is affected.
pub fn render_pdf(document: &VectorDocument) -> CadResult<Vec<u8>> {
    validate_document(document)?;
    let points_per_mm = 72.0 / MM_PER_INCH;
    let width_pt = document.width_mm * points_per_mm;
    let height_pt = document.height_mm * points_per_mm;

    // One ExtGState per distinct alpha actually used, keyed on the quantised
    // value so the writer stays deterministic and does not emit unused objects.
    let mut alphas: Vec<f32> = Vec::new();
    for path in &document.paths {
        let alpha = quantised_alpha(path.opacity);
        if alpha < 1.0 && !alphas.contains(&alpha) {
            alphas.push(alpha);
        }
    }
    alphas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut content: Vec<u8> = Vec::new();
    content.extend_from_slice(
        format!(
            "q\n{} 0 0 {} 0 0 cm\n",
            number(points_per_mm),
            number(points_per_mm)
        )
        .as_bytes(),
    );
    for path in &document.paths {
        push_pdf_path(&mut content, path, &alphas);
    }
    content.extend_from_slice(b"Q\n");

    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
    ];

    // Objects 5.. are the ExtGState resources; object 4 is the page content.
    // The page object references them through `/Resources /ExtGState`, so the
    // transparency is a real PDF resource and not an ignored content operator.
    let ext_entries: Vec<String> = alphas
        .iter()
        .enumerate()
        .map(|(index, _alpha)| format!("/GS{index} {} 0 R", 5 + index))
        .collect();
    let resources = if ext_entries.is_empty() {
        "<< >>".to_string()
    } else {
        let mut entries = ext_entries.clone();
        entries.push(format!("/GS_opaque {} 0 R", 5 + alphas.len()));
        format!("<< /ExtGState << {} >> >>", entries.join(" "))
    };
    objects.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents 4 0 R /Resources {resources} >>",
            number(width_pt),
            number(height_pt)
        )
        .into_bytes(),
    );
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(&content);
    stream.extend_from_slice(b"endstream");
    objects.push(stream);
    for alpha in &alphas {
        objects.push(
            format!(
                "<< /Type /ExtGState /ca {} /CA {} >>",
                number(*alpha as f64),
                number(*alpha as f64)
            )
            .into_bytes(),
        );
    }
    if !alphas.is_empty() {
        // The final resource restores fully opaque alpha after a translucent path.
        objects.push(b"<< /Type /ExtGState /ca 1 /CA 1 >>".to_vec());
    }

    let mut buffer: Vec<u8> = Vec::new();
    buffer.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets: Vec<usize> = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(buffer.len());
        buffer.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        buffer.extend_from_slice(body);
        buffer.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = buffer.len();
    let count = objects.len();
    buffer.extend_from_slice(format!("xref\n0 {}\n", count + 1).as_bytes());
    buffer.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        buffer.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    buffer.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            count + 1,
            xref_offset
        )
        .as_bytes(),
    );
    Ok(buffer)
}

/// Clamp opacity into `[0, 1]` and quantise it so identical alphas share one
/// resource; a non-finite value is validated away before this point.
fn quantised_alpha(opacity: f32) -> f32 {
    let clamped = opacity.clamp(0.0, 1.0);
    (clamped * 1000.0).round() / 1000.0
}

fn push_pdf_path(content: &mut Vec<u8>, path: &VectorPath, alphas: &[f32]) {
    let alpha = quantised_alpha(path.opacity);
    if alpha < 1.0 {
        // The page resources are emitted by index in ascending alpha order.
        let index = alphas
            .iter()
            .position(|candidate| *candidate == alpha)
            .expect("alpha collected during the pre-pass");
        content.extend_from_slice(format!("/GS{index} gs\n").as_bytes());
    }
    if let Some(stroke) = path.stroke {
        content.extend_from_slice(format!("{} RG\n", pdf_components(stroke)).as_bytes());
        content.extend_from_slice(format!("{} w\n", number(path.stroke_width_mm)).as_bytes());
    }
    if let Some(fill) = path.fill {
        content.extend_from_slice(format!("{} rg\n", pdf_components(fill)).as_bytes());
    }
    let first = path.points[0];
    content.extend_from_slice(format!("{} {} m\n", number(first[0]), number(first[1])).as_bytes());
    for point in &path.points[1..] {
        content
            .extend_from_slice(format!("{} {} l\n", number(point[0]), number(point[1])).as_bytes());
    }
    if path.closed {
        content.extend_from_slice(b"h\n");
    }
    let operator = match (path.fill.is_some(), path.stroke.is_some()) {
        (true, true) => "B",
        (true, false) => "f",
        (false, true) => "S",
        (false, false) => "n",
    };
    content.extend_from_slice(format!("{operator}\n").as_bytes());
    if alpha < 1.0 {
        // Restore opaque graphics state for whatever follows.
        content.extend_from_slice(b"/GS_opaque gs\n");
    }
}

// ---------------------------------------------------------------------------
// Stable database-facing entry points
// ---------------------------------------------------------------------------

/// Render one paper-space layout to SVG bytes, purely on the CPU.
///
/// This is the stable entry point for vector export: it resolves the layout's
/// [`cad_db::PlotSettingsRecord`], plans the page (honouring margins, rotation
/// and scale), builds the paper-space representation through the provider
/// registry (expanding INSERTs and applying viewport transforms) and serialises
/// SVG. It neither creates nor touches a GPU device.
pub fn render_plot_svg(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    layout: LayoutId,
    target: PlotTarget,
    context: &RepresentationContext,
) -> CadResult<Vec<u8>> {
    let record = database.plot_settings_for(layout);
    let page = plan_plot_for_record(&record, target)?;
    let representation = build_paper_space(registry, database, layout, context, &|_| true)?;
    let document = build_vector_document(&representation, &page)?;
    render_svg(&document)
}

/// Render one paper-space layout to PDF bytes, purely on the CPU.
///
/// See [`render_plot_svg`]; the only difference is the serialisation target.
pub fn render_plot_pdf(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    layout: LayoutId,
    target: PlotTarget,
    context: &RepresentationContext,
) -> CadResult<Vec<u8>> {
    let record = database.plot_settings_for(layout);
    let page = plan_plot_for_record(&record, target)?;
    let representation = build_paper_space(registry, database, layout, context, &|_| true)?;
    let document = build_vector_document(&representation, &page)?;
    render_pdf(&document)
}

// ---------------------------------------------------------------------------
// Shared formatting/validation helpers
// ---------------------------------------------------------------------------

fn validate_document(document: &VectorDocument) -> CadResult<()> {
    if !document.width_mm.is_finite()
        || !document.height_mm.is_finite()
        || document.width_mm <= 0.0
        || document.height_mm <= 0.0
    {
        return Err(CadError::InvalidInput(format!(
            "vector document size must be positive and finite, got {}x{} mm",
            document.width_mm, document.height_mm
        )));
    }
    for path in &document.paths {
        if path.points.len() < 2 {
            return Err(CadError::InvalidInput(
                "vector path has fewer than two points".into(),
            ));
        }
        if !path.stroke_width_mm.is_finite() || path.stroke_width_mm < 0.0 {
            return Err(CadError::InvalidInput(
                "vector path has an invalid stroke width".into(),
            ));
        }
        if !path.opacity.is_finite() {
            return Err(CadError::InvalidInput(
                "vector path has a non-finite opacity".into(),
            ));
        }
        if path.fill.is_none() && path.stroke.is_none() {
            return Err(CadError::InvalidInput(
                "vector path has neither fill nor stroke".into(),
            ));
        }
        for point in &path.points {
            if !point[0].is_finite() || !point[1].is_finite() {
                return Err(CadError::InvalidInput(
                    "vector path has a non-finite coordinate".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Deterministic decimal formatting: fixed notation, no exponent, trimmed.
///
/// Finite values only; callers validate first. `-0` is normalised to `0`.
fn number(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let mut text = format!("{value:.6}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

fn hex_color(color: [f32; 3]) -> String {
    let channel = |value: f32| -> u32 { (value.clamp(0.0, 1.0) * 255.0).round() as u32 };
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color[0]),
        channel(color[1]),
        channel(color[2])
    )
}

fn pdf_components(color: [f32; 3]) -> String {
    let component = |value: f32| number(value.clamp(0.0, 1.0) as f64);
    format!(
        "{} {} {}",
        component(color[0]),
        component(color[1]),
        component(color[2])
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{
        LinetypePattern, PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation,
        PlotSettingsRecord, PlotType,
    };
    use cad_domain::{DocumentId, EntityId, GeometrySource, InstancePath, Precision, SelectionRef};
    use std::sync::Arc;

    use crate::{DisplayFragment, Mesh, PlotScale};

    fn paper(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn fragment(primitive: DisplayPrimitive) -> DisplayFragment {
        DisplayFragment {
            source: SelectionRef {
                document: DocumentId(1),
                entity: EntityId(7),
                instance: InstancePath::default(),
                sub_element: None,
            },
            geometry_source: GeometrySource::Analytic,
            precision: Precision::Analytic,
            alpha: 1.0,
            color: [0.0, 0.0, 0.0],
            color_unresolved: false,
            lineweight: 0.25,
            lineweight_unresolved: false,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: false,
            linetype_scale: 1.0,
            primitive,
        }
    }

    fn lines(points: Vec<Point3>) -> DisplayFragment {
        fragment(DisplayPrimitive::Lines(Arc::from(
            points.into_boxed_slice(),
        )))
    }

    fn representation(fragments: Vec<DisplayFragment>) -> DisplayRepresentation {
        DisplayRepresentation {
            fragments,
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    /// A 100 x 100 mm page at 1 px/mm with the planner's y-down pixel map.
    fn simple_page() -> PlotPage {
        PlotPage {
            width: 100,
            height: 100,
            dpi: MM_PER_INCH,
            pixels_per_mm: 1.0,
            rotation_degrees: 0.0,
            printable_mm: (100.0, 100.0),
            paper_to_pixel: [1.0, 0.0, 0.0, -1.0, 0.0, 100.0],
        }
    }

    fn a4(rotation: PlotRotation) -> PlotSettingsRecord {
        PlotSettingsRecord {
            layout: LayoutId(1),
            paper_size_name: "ISO_A4_(210.00_x_297.00_MM)".into(),
            paper_width: 210.0,
            paper_height: 297.0,
            margins: PlotMargins::default(),
            rotation,
            scale_numerator: 1.0,
            scale_denominator: 1.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::Imported,
        }
    }

    // ---- page transform ----------------------------------------------------

    #[test]
    fn vector_page_is_y_up_millimetres() {
        let page = VectorPage::from_plot_page(&simple_page()).unwrap();
        assert_eq!(page.width_mm, 100.0);
        assert_eq!(page.height_mm, 100.0);
        // Paper origin (y-up bottom-left) maps to the document origin.
        let origin = page.map_paper_point(paper(0.0, 0.0));
        assert!(origin[0].abs() < 1e-9 && origin[1].abs() < 1e-9);
        // Paper top-right maps to the document top-right.
        let top_right = page.map_paper_point(paper(100.0, 100.0));
        assert!((top_right[0] - 100.0).abs() < 1e-9);
        assert!((top_right[1] - 100.0).abs() < 1e-9);
        let quarter = page.map_paper_point(paper(25.0, 40.0));
        assert!((quarter[0] - 25.0).abs() < 1e-9);
        assert!((quarter[1] - 40.0).abs() < 1e-9);
    }

    #[test]
    fn rotation_90_swaps_the_document_size() {
        // A4 portrait rotated 90° becomes landscape; at 254 dpi (10 px/mm).
        let page = crate::plan_plot(
            &a4(PlotRotation::Degrees90),
            PlotScale::ONE_TO_ONE,
            PlotTarget::Dpi(254.0),
        )
        .unwrap();
        let vector = VectorPage::from_plot_page(&page).unwrap();
        assert!(
            (vector.width_mm - 297.0).abs() < 1e-9,
            "{}",
            vector.width_mm
        );
        assert!(
            (vector.height_mm - 210.0).abs() < 1e-9,
            "{}",
            vector.height_mm
        );
        assert!(vector.width_mm > vector.height_mm);
    }

    #[test]
    fn physical_size_is_dpi_independent() {
        let at_100 = VectorPage::from_plot_page(
            &crate::plan_plot(
                &a4(PlotRotation::None),
                PlotScale::ONE_TO_ONE,
                PlotTarget::Dpi(100.0),
            )
            .unwrap(),
        )
        .unwrap();
        let at_600 = VectorPage::from_plot_page(
            &crate::plan_plot(
                &a4(PlotRotation::None),
                PlotScale::ONE_TO_ONE,
                PlotTarget::Dpi(600.0),
            )
            .unwrap(),
        )
        .unwrap();
        // The two differ only by the plan's integer-canvas rounding, which is at
        // most one pixel divided by the DPI: 1/3.937 ≈ 0.254 mm at 100 dpi.
        assert!((at_100.width_mm - at_600.width_mm).abs() < 0.5);
        assert!((at_100.height_mm - at_600.height_mm).abs() < 0.5);
    }

    // ---- SVG ---------------------------------------------------------------

    #[test]
    fn svg_document_has_xml_header_and_svg_root() {
        let page = VectorPage::from_plot_page(&simple_page()).unwrap();
        let bytes = render_svg(&VectorDocument::empty(&page)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(text.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(text.contains("width=\"100mm\" height=\"100mm\" viewBox=\"0 0 100 100\""));
        assert!(text.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn svg_flips_y_and_writes_points_in_millimetres() {
        let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)])]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
        // y-up document (0,0) → SVG y-down (0,100).
        assert!(text.contains("points=\"0,100 10,100\""), "{text}");
        assert!(text.contains("stroke-width=\"0.25\""), "{text}");
        assert!(text.contains("fill=\"none\""), "{text}");
    }

    #[test]
    fn svg_is_self_contained() {
        let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(5.0, 5.0)])]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
        // No external references: the only URLs are the SVG namespace itself.
        assert!(!text.contains("href="), "{text}");
        assert!(!text.contains("<script"), "{text}");
    }

    // ---- PDF ---------------------------------------------------------------

    #[test]
    fn pdf_document_has_header_xref_and_eof() {
        let page = VectorPage::from_plot_page(&simple_page()).unwrap();
        let bytes = render_pdf(&VectorDocument::empty(&page)).unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.contains("1 0 obj"), "{text}");
        assert!(text.contains("/Type /Catalog"), "{text}");
        assert!(text.contains("xref"), "{text}");
        assert!(text.contains("startxref"), "{text}");
        assert!(text.trim_end().ends_with("%%EOF"));
        // An opaque empty page has the free entry plus catalog/pages/page/content.
        assert!(text.contains("xref\n0 5\n"), "{text}");
    }

    #[test]
    fn pdf_media_box_matches_the_physical_page() {
        let page = VectorPage::from_plot_page(&simple_page()).unwrap();
        let bytes = render_pdf(&VectorDocument::empty(&page)).unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // 100 mm = 283.464567 pt.
        assert!(
            text.contains("/MediaBox [0 0 283.464567 283.464567]"),
            "{text}"
        );
    }

    #[test]
    fn pdf_writes_path_operators_and_length() {
        let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)])]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        let text = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
        assert!(text.contains(" m\n"), "{text}");
        assert!(text.contains(" l\n"), "{text}");
        assert!(text.contains("S\n"), "{text}");
        // The stream length must equal the content byte count (non-zero here).
        assert!(text.contains("/Length "), "{text}");
    }

    // ---- empty page --------------------------------------------------------

    #[test]
    fn empty_page_is_a_valid_document_with_no_paths() {
        let representation = representation(Vec::new());
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(document.completeness, Completeness::Complete);
        assert!(document.diagnostics.is_empty());
        assert!(!render_svg(&document).unwrap().is_empty());
        assert!(!render_pdf(&document).unwrap().is_empty());
    }

    // ---- unsupported primitives -------------------------------------------

    #[test]
    fn unshaped_text_is_reported_not_dropped() {
        let representation = representation(vec![fragment(DisplayPrimitive::Text {
            text: "hello".into(),
            origin: paper(0.0, 0.0),
            font: cad_resources::ResourceKey("style:Standard".into()),
            height: 2.5,
        })]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(document.diagnostics.len(), 1);
        assert_eq!(
            document.diagnostics[0].code,
            vector_diagnostic::TEXT_UNSHAPED
        );
        assert_eq!(
            document.completeness,
            Completeness::Partial(vec![document.diagnostics[0].message.clone()])
        );
    }

    #[test]
    fn image_is_reported_not_dropped() {
        let representation = representation(vec![fragment(DisplayPrimitive::Image {
            resource: cad_resources::ResourceKey("img:0".into()),
            transform: cad_domain::Transform3::identity(),
        })]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(
            document.diagnostics[0].code,
            vector_diagnostic::IMAGE_UNSUPPORTED
        );
    }

    #[test]
    fn unexpanded_instance_is_reported_not_dropped() {
        let representation = representation(vec![fragment(DisplayPrimitive::Instance {
            block: cad_domain::BlockId(1),
            transform: cad_domain::Transform3::identity(),
        })]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(
            document.diagnostics[0].code,
            vector_diagnostic::UNEXPANDED_INSTANCE
        );
    }

    #[test]
    fn mesh_without_triangles_is_reported() {
        let representation = representation(vec![fragment(DisplayPrimitive::Mesh(Arc::new(
            Mesh::default(),
        )))]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(
            document.diagnostics[0].code,
            vector_diagnostic::MESH_WITHOUT_TRIANGLES
        );
    }

    #[test]
    fn mesh_triangles_become_filled_polygons() {
        let mesh = Mesh {
            vertices: vec![paper(0.0, 0.0), paper(10.0, 0.0), paper(0.0, 10.0)],
            triangles: vec![[0, 1, 2]],
            ..Mesh::default()
        };
        let representation = representation(vec![fragment(DisplayPrimitive::Mesh(Arc::new(mesh)))]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert_eq!(document.paths.len(), 1);
        assert!(document.paths[0].closed);
        assert_eq!(document.paths[0].fill, Some([0.0, 0.0, 0.0]));
        assert_eq!(document.paths[0].points.len(), 3);
    }

    // ---- degenerate / non-finite ------------------------------------------

    #[test]
    fn degenerate_polyline_is_reported() {
        let representation = representation(vec![lines(vec![paper(1.0, 1.0)])]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(
            document.diagnostics[0].code,
            vector_diagnostic::DEGENERATE_POLYLINE
        );
    }

    #[test]
    fn non_finite_polyline_is_reported() {
        let representation =
            representation(vec![lines(vec![paper(0.0, 0.0), paper(f64::NAN, 1.0)])]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        assert!(document.paths.is_empty());
        assert_eq!(document.diagnostics[0].code, vector_diagnostic::NON_FINITE);
    }

    #[test]
    fn opacity_below_one_is_carried_to_svg() {
        let mut f = lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)]);
        f.alpha = 0.5;
        let representation = representation(vec![f]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
        assert!(text.contains("stroke-opacity=\"0.5\""), "{text}");
    }

    #[test]
    fn pdf_opacity_becomes_an_extgstate_resource() {
        let mut f = lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)]);
        f.alpha = 0.25;
        let representation = representation(vec![f]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        let text = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
        // A real ExtGState resource, referenced from the page and selected with
        // `gs`, not an ignored content operator.
        assert!(
            text.contains("/ExtGState << /GS0 5 0 R /GS_opaque 6 0 R >>"),
            "{text}"
        );
        assert!(
            text.contains("/Type /ExtGState /ca 0.25 /CA 0.25"),
            "{text}"
        );
        assert!(text.contains("/GS0 gs"), "{text}");
        assert!(text.contains("/GS_opaque gs"), "{text}");
        assert!(text.contains("xref\n0 7\n"), "{text}");
    }

    #[test]
    fn fully_opaque_pdf_has_no_transparency_objects() {
        let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)])]);
        let document = build_vector_document(&representation, &simple_page()).unwrap();
        let text = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
        assert!(!text.contains("/ExtGState"), "{text}");
        assert!(!text.contains(" gs\n"), "{text}");
    }
}
