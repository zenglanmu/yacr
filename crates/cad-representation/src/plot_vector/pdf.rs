use cad_domain::CadResult;

use crate::MM_PER_INCH;

use super::format::{number, pdf_components, validate_document};
use super::model::{VectorDocument, VectorPath};

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
