use cad_domain::CadResult;

use super::format::{hex_color, number, validate_document};
use super::model::{VectorDocument, VectorPath};

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
