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
    let fill = match (path.closed, path.fill) {
        // A filled polygon uses the even-odd rule, the same rule the raster
        // HATCH and WIPEOUT fills use: a self-intersecting ring is filled as
        // drawn, never repaired or reinterpreted by the nonzero rule, and every
        // extra ring is a hole (`<path>` with several subpaths).
        (true, Some(color)) => hex_color(color),
        _ => "none".to_string(),
    };
    let geometry = if path.extra_rings.is_empty() {
        // One ring: keep the compact element form (`<polygon>`/`<polyline>`).
        let element = if path.closed { "polygon" } else { "polyline" };
        let points: Vec<String> = path
            .points
            .iter()
            .map(|point| svg_point(*point, height_mm))
            .collect();
        format!("<{element} points=\"{}\"", points.join(" "))
    } else {
        // Several rings: one `<path>` whose subpaths share the even-odd fill, so
        // a ring inside another one really is a hole (an inverted WIPEOUT's page
        // rectangle minus its boundary).
        let subpaths: Vec<String> = std::iter::once(&path.points)
            .chain(path.extra_rings.iter())
            .map(|ring| svg_subpath(ring, height_mm))
            .collect();
        format!("<path d=\"{}\"", subpaths.join(" "))
    };
    let mut attrs = format!("{geometry} fill=\"{fill}\"");
    if path.closed && path.fill.is_some() {
        attrs.push_str(" fill-rule=\"evenodd\"");
    }
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
    let element = if path.extra_rings.is_empty() {
        if path.closed {
            "polygon"
        } else {
            "polyline"
        }
    } else {
        "path"
    };
    format!("  <{element} {attrs}/>\n")
}

/// One SVG point in the writer's y-down user space.
fn svg_point(point: [f64; 2], height_mm: f64) -> String {
    format!("{},{}", number(point[0]), number(height_mm - point[1]))
}

/// One closed SVG subpath: `M` for the first point, `L` for the rest, `Z`.
fn svg_subpath(ring: &[[f64; 2]], height_mm: f64) -> String {
    let mut data = String::new();
    for (index, point) in ring.iter().enumerate() {
        if index > 0 {
            data.push(' ');
        }
        data.push(if index == 0 { 'M' } else { 'L' });
        data.push_str(&svg_point(*point, height_mm));
    }
    data.push_str(" Z");
    data
}
