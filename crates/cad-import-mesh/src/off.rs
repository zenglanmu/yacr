//! OFF (Object File Format) reader.
//!
//! ASCII only. The plain `OFF` and colour `COFF` variants are read; every other
//! OFF-family prefix (`NOFF`, `CNOFF`, `STOFF`, `4OFF`, ...) is refused by name
//! with [`ERR_VARIANT_REFUSED`] rather than falling through to a generic error.
//!
//! The layout is:
//!
//! ```text
//! OFF
//! numVertices numFaces numEdges
//! x y z [r g b [a]]                 (per vertex)
//! n i0 i1 ... i(n-1) [faceColour]   (per face)
//! ```
//!
//! `numEdges` is accepted and ignored (it is not per-face data this reader uses).
//! A face with `n > 3` is fan-triangulated and reported ([`DIAG_NGON_FAN`] +
//! `Partial`); `n < 3` is reported and skipped. Indices outside the declared
//! vertex count, non-finite coordinates, fewer lines than the declared counts and
//! trailing non-comment data are all [`CadError::CorruptData`], never a silent
//! partial success.

use cad_domain::{CadError, CadResult, Point3};
use cad_import_core::{
    compute_identity, ImportPhase, ImportProgress, ImportProgressSink, ImportRequest,
    ImportedDrawing, Importer, NoopProgress, OFF_FAMILY_PREFIXES,
};

use crate::{publish, Note, ParsedMesh};

/// Stable diagnostic code: an n-gon was fan-triangulated.
pub const DIAG_NGON_FAN: &str = "import.off.ngon_fan_triangulated";
/// Stable diagnostic code: a face had fewer than three corners and was skipped.
pub const DIAG_FACE_TOO_FEW_VERTICES: &str = "import.off.face_too_few_vertices";
/// Stable diagnostic code: a degenerate triangle was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.off.degenerate_triangle";
/// Stable diagnostic code: declared vertex colours were unusable and dropped.
pub const DIAG_COLORS_DROPPED: &str = "import.off.colors_dropped";
/// Stable diagnostic code: an alpha channel was present but is not stored.
pub const DIAG_ALPHA_DROPPED: &str = "import.off.alpha_dropped";
/// Stable diagnostic code: face colours are present but the mesh has no channel.
pub const DIAG_FACE_COLOR_NOT_IMPORTED: &str = "import.off.face_color_not_imported";

/// Stable refusal code: the file is not ASCII.
pub const ERR_NON_ASCII: &str = "import.off.non_ascii";
/// Stable refusal code: a known but unsupported OFF variant prefix.
pub const ERR_VARIANT_REFUSED: &str = "import.off.variant_refused";
/// Stable refusal code: the OFF grammar is malformed.
pub const ERR_MALFORMED_HEADER: &str = "import.off.malformed_header";
/// Stable refusal code: fewer lines than the declared counts.
pub const ERR_TRUNCATED: &str = "import.off.truncated_payload";
/// Stable refusal code: non-comment data follows the declared elements.
pub const ERR_TRAILING_DATA: &str = "import.off.trailing_data";
/// Stable refusal code: a vertex coordinate is not finite.
pub const ERR_NON_FINITE: &str = "import.off.non_finite_coordinate";
/// Stable refusal code: a face index is outside the vertex list.
pub const ERR_INDEX_RANGE: &str = "import.off.face_index_out_of_range";

/// The OFF entity `type_key` reported in capabilities and stored rows.
pub const OFF_MESH_TYPE_KEY: &str = "OffMesh";

/// The OFF importer.
#[derive(Default)]
pub struct OffImporter;

impl OffImporter {
    pub fn new() -> Self {
        Self
    }
}

impl Importer for OffImporter {
    fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<ImportedDrawing> {
        self.import_with_progress(request, cancelled, &NoopProgress)
    }

    fn import_with_progress(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn ImportProgressSink,
    ) -> CadResult<ImportedDrawing> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if request.bytes.len() > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "file is {} bytes, over the {} byte import limit",
                request.bytes.len(),
                request.limits.max_file_bytes
            )));
        }
        if request.limits.max_entities == 0 {
            return Err(CadError::InvalidInput(
                "OFF is one mesh entity, over the 0 entity import limit".into(),
            ));
        }

        let started = web_time::Instant::now();
        let identity = compute_identity(&request.bytes);
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });

        let parsed = parse(&request.bytes, cancelled)?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        progress.report(ImportProgress::at(ImportPhase::Parsing, 0, Some(1)));

        publish(
            request,
            progress,
            identity,
            started,
            parsed,
            OFF_MESH_TYPE_KEY,
            "OFF",
        )
    }
}

fn malformed(message: impl std::fmt::Display) -> CadError {
    CadError::CorruptData(format!("{ERR_MALFORMED_HEADER}: {message}"))
}

struct OffLine<'a> {
    number: usize,
    text: &'a str,
}

/// Split into non-blank, comment-stripped lines. Comments (`#`) may appear on
/// their own or after data; blank lines are ignored.
fn lines(bytes: &[u8]) -> CadResult<Vec<OffLine<'_>>> {
    if !bytes.is_ascii() {
        return Err(CadError::CorruptData(format!(
            "{ERR_NON_ASCII}: OFF is an ASCII format; the input contains non-ASCII bytes"
        )));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CadError::CorruptData(format!("{ERR_NON_ASCII}: input is not UTF-8")))?;
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        out.push(OffLine {
            number: index + 1,
            text: content,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Plain,
    Color,
}

fn parse_variant(line: &OffLine<'_>) -> CadResult<Variant> {
    let mut tokens = line.text.split_whitespace();
    let Some(prefix) = tokens.next() else {
        return Err(malformed("empty variant line"));
    };
    if tokens.next().is_some() {
        return Err(malformed(format!(
            "line {}: the variant line must contain only 'OFF' or 'COFF'",
            line.number
        )));
    }
    match prefix {
        "OFF" => Ok(Variant::Plain),
        "COFF" => Ok(Variant::Color),
        other if OFF_FAMILY_PREFIXES.contains(&other) => Err(CadError::CorruptData(format!(
            "{ERR_VARIANT_REFUSED}: OFF variant '{other}' is not supported; only OFF and COFF are read"
        ))),
        other => Err(malformed(format!(
            "line {}: unsupported OFF variant '{other}'",
            line.number
        ))),
    }
}

fn parse_count(token: &str, line: usize, field: &str) -> CadResult<usize> {
    token.parse::<usize>().map_err(|_| {
        malformed(format!(
            "line {line}: {field} '{token}' is not a non-negative integer"
        ))
    })
}

fn coordinate(token: &str, line: usize, axis: &str) -> CadResult<f64> {
    let value = token.parse::<f64>().map_err(|_| {
        malformed(format!(
            "line {line}: {axis} coordinate '{token}' is not a number"
        ))
    })?;
    if !value.is_finite() {
        return Err(CadError::CorruptData(format!(
            "{ERR_NON_FINITE}: line {line} has a non-finite {axis} coordinate"
        )));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy)]
enum ColorDecision {
    Color([u8; 3]),
    Ambiguous,
    Invalid,
}

/// Decide whether a colour triple is integer 0-255 or float 0-1.
///
/// A triple whose every value is an integer 0/1 is genuinely ambiguous (both
/// readings are valid), so it is dropped and reported instead of guessed. A
/// fractional triple is float if every channel is in `[0, 1]`; integer values
/// above 1 are 0-255. Anything else is invalid.
fn decide_color(values: [f64; 3]) -> ColorDecision {
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0 || *value > 255.0)
    {
        return ColorDecision::Invalid;
    }
    let all_integral = values.iter().all(|value| value.fract() == 0.0);
    if all_integral {
        if values.iter().any(|value| *value > 1.0) {
            return ColorDecision::Color([values[0] as u8, values[1] as u8, values[2] as u8]);
        }
        return ColorDecision::Ambiguous;
    }
    if values.iter().all(|value| *value <= 1.0) {
        return ColorDecision::Color([
            (values[0] * 255.0).round() as u8,
            (values[1] * 255.0).round() as u8,
            (values[2] * 255.0).round() as u8,
        ]);
    }
    ColorDecision::Invalid
}

fn parse(bytes: &[u8], cancelled: &dyn Fn() -> bool) -> CadResult<ParsedMesh> {
    let lines = lines(bytes)?;
    let mut cursor = 0usize;
    let variant_line = lines
        .get(cursor)
        .ok_or_else(|| malformed("empty file, expected an 'OFF'/'COFF' variant line"))?;
    parse_variant(variant_line)?;
    cursor += 1;

    let counts_line = lines.get(cursor).ok_or_else(|| {
        CadError::CorruptData(format!(
            "{ERR_TRUNCATED}: file ended before the counts line"
        ))
    })?;
    let counts: Vec<&str> = counts_line.text.split_whitespace().collect();
    if counts.len() != 3 {
        return Err(malformed(format!(
            "line {}: counts line must be 'numVertices numFaces numEdges'",
            counts_line.number
        )));
    }
    let vertex_count = parse_count(counts[0], counts_line.number, "vertex count")?;
    let face_count = parse_count(counts[1], counts_line.number, "face count")?;
    // Declared, deliberately unused: OFF edge lists are not part of this mesh.
    let _edge_count = parse_count(counts[2], counts_line.number, "edge count")?;
    cursor += 1;

    if vertex_count > u32::MAX as usize {
        return Err(malformed(format!(
            "vertex count {vertex_count} exceeds the u32 mesh index range"
        )));
    }

    let remaining = lines.len().saturating_sub(cursor);
    if vertex_count.saturating_add(face_count) > remaining {
        return Err(CadError::CorruptData(format!(
            "{ERR_TRUNCATED}: file declares {vertex_count} vertex line(s) and {face_count} face line(s), but only {remaining} data line(s) remain"
        )));
    }

    let mut vertices = Vec::with_capacity(vertex_count);
    let mut colors: Option<Vec<[u8; 3]>> = None;
    let mut colored_vertices = 0usize;
    let mut ambiguous_colors = 0usize;
    let mut invalid_colors = 0usize;
    let mut alpha_count = 0usize;

    for _ in 0..vertex_count {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        let line = &lines[cursor];
        cursor += 1;
        let tokens: Vec<&str> = line.text.split_whitespace().collect();
        if !matches!(tokens.len(), 3 | 6 | 7) {
            return Err(malformed(format!(
                "line {}: vertex must be 'x y z [r g b [a]]', found {} token(s)",
                line.number,
                tokens.len()
            )));
        }
        let point = Point3 {
            x: coordinate(tokens[0], line.number, "x")?,
            y: coordinate(tokens[1], line.number, "y")?,
            z: coordinate(tokens[2], line.number, "z")?,
        };
        vertices.push(point);

        if tokens.len() >= 6 {
            colored_vertices += 1;
            let channels = [
                coordinate(tokens[3], line.number, "red")?,
                coordinate(tokens[4], line.number, "green")?,
                coordinate(tokens[5], line.number, "blue")?,
            ];
            if tokens.len() == 7 {
                // Alpha is validated as a number, then dropped (not stored).
                let _alpha = coordinate(tokens[6], line.number, "alpha")?;
                alpha_count += 1;
            }
            match decide_color(channels) {
                ColorDecision::Color(color) => match colors.as_mut() {
                    Some(colors) => colors.push(color),
                    None => colors = Some(vec![color]),
                },
                ColorDecision::Ambiguous => ambiguous_colors += 1,
                ColorDecision::Invalid => invalid_colors += 1,
            }
        }
    }

    let mut color_note = None;
    if ambiguous_colors > 0 || invalid_colors > 0 {
        color_note = Some(format!(
            "{ambiguous_colors} vertex colour triple(s) were ambiguous (integer 0/1 can mean 0-255 or 0-1) and {invalid_colors} were invalid (outside 0-255 or 0-1); vertex colours were dropped"
        ));
        colors = None;
    } else if colored_vertices > 0 && colored_vertices != vertex_count {
        color_note =
            Some("not every vertex declared a colour; vertex colours were dropped".to_string());
        colors = None;
    }

    let remaining = lines.len().saturating_sub(cursor);
    if face_count > remaining {
        return Err(CadError::CorruptData(format!(
            "{ERR_TRUNCATED}: file declares {face_count} face line(s) but only {remaining} data line(s) remain"
        )));
    }

    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut ngon_faces = 0usize;
    let mut short_faces = 0usize;
    let mut repeated_index_triangles = 0usize;
    let mut face_colors = 0usize;

    for _ in 0..face_count {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        let line = &lines[cursor];
        cursor += 1;
        let tokens: Vec<&str> = line.text.split_whitespace().collect();
        let corners = parse_count(tokens[0], line.number, "face corner count")?;
        let indices = &tokens[1..];
        if indices.len() < corners {
            return Err(CadError::CorruptData(format!(
                "{ERR_TRUNCATED}: line {} declares {corners} face corner(s) but only {} token(s) follow",
                line.number,
                indices.len()
            )));
        }
        if indices.len() > corners + 1 {
            return Err(malformed(format!(
                "line {}: face has {} trailing token(s); at most one face colour token is supported",
                line.number,
                indices.len() - corners - 1
            )));
        }
        if indices.len() == corners + 1 {
            // Validate the face colour as an integer index, then drop it: the
            // neutral mesh has no per-face colour channel.
            indices[corners].parse::<i64>().map_err(|_| {
                malformed(format!(
                    "line {}: face colour token '{}' is not an integer",
                    line.number, indices[corners]
                ))
            })?;
            face_colors += 1;
        }

        let mut face = Vec::with_capacity(corners);
        for token in &indices[..corners] {
            let index = token.parse::<i64>().map_err(|_| {
                malformed(format!(
                    "line {}: face index '{token}' is not an integer",
                    line.number
                ))
            })?;
            if index < 0 || index as u64 >= vertex_count as u64 {
                return Err(CadError::CorruptData(format!(
                    "{ERR_INDEX_RANGE}: line {} face index {index} is outside the {vertex_count}-vertex list",
                    line.number
                )));
            }
            face.push(index as u32);
        }

        match face.len() {
            0..=2 => short_faces += 1,
            3 => push_triangle(
                [face[0], face[1], face[2]],
                &mut triangles,
                &mut repeated_index_triangles,
            ),
            count => {
                ngon_faces += 1;
                for offset in 1..count - 1 {
                    push_triangle(
                        [face[0], face[offset], face[offset + 1]],
                        &mut triangles,
                        &mut repeated_index_triangles,
                    );
                }
            }
        }
    }

    if cursor != lines.len() {
        let line = &lines[cursor];
        let snippet: String = line.text.chars().take(40).collect();
        return Err(CadError::CorruptData(format!(
            "{ERR_TRAILING_DATA}: line {} is data after the declared counts: '{snippet}'",
            line.number
        )));
    }

    let mut degenerate = repeated_index_triangles;
    let mut kept = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        if is_zero_area(triangle, &vertices) {
            degenerate += 1;
            continue;
        }
        kept.push(triangle);
    }

    let mut notes = Vec::new();
    if ngon_faces > 0 {
        notes.push(Note {
            code: DIAG_NGON_FAN,
            message: format!(
                "{ngon_faces} face(s) with more than 3 vertices were fan-triangulated; concave n-gons may be incorrect"
            ),
            partial: true,
        });
    }
    if short_faces > 0 {
        notes.push(Note {
            code: DIAG_FACE_TOO_FEW_VERTICES,
            message: format!("{short_faces} face(s) had fewer than 3 vertices and were skipped"),
            partial: true,
        });
    }
    if degenerate > 0 {
        notes.push(Note {
            code: DIAG_DEGENERATE_TRIANGLE,
            message: format!(
                "{degenerate} degenerate triangle(s) (repeated vertex or zero area) were dropped"
            ),
            partial: true,
        });
    }
    if let Some(message) = color_note {
        notes.push(Note {
            code: DIAG_COLORS_DROPPED,
            message,
            partial: false,
        });
    }
    if alpha_count > 0 {
        notes.push(Note {
            code: DIAG_ALPHA_DROPPED,
            message: format!(
                "{alpha_count} vertex alpha channel(s) were present but are not stored; only RGB is kept"
            ),
            partial: false,
        });
    }
    if face_colors > 0 {
        notes.push(Note {
            code: DIAG_FACE_COLOR_NOT_IMPORTED,
            message: format!(
                "{face_colors} face colour token(s) are not imported; the neutral mesh has no per-face colour channel"
            ),
            partial: false,
        });
    }

    Ok(ParsedMesh {
        vertices,
        triangles: kept,
        normals: Vec::new(),
        colors: colors.unwrap_or_default(),
        notes,
    })
}

fn push_triangle(triangle: [u32; 3], triangles: &mut Vec<[u32; 3]>, repeated: &mut usize) {
    if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
        *repeated += 1;
        return;
    }
    triangles.push(triangle);
}

fn is_zero_area(triangle: [u32; 3], vertices: &[Point3]) -> bool {
    let p0 = vertices[triangle[0] as usize];
    let p1 = vertices[triangle[1] as usize];
    let p2 = vertices[triangle[2] as usize];
    let ux = p1.x - p0.x;
    let uy = p1.y - p0.y;
    let uz = p1.z - p0.z;
    let vx = p2.x - p0.x;
    let vy = p2.y - p0.y;
    let vz = p2.z - p0.z;
    let cx = uy * vz - uz * vy;
    let cy = uz * vx - ux * vz;
    let cz = ux * vy - uy * vx;
    cx == 0.0 && cy == 0.0 && cz == 0.0
}
