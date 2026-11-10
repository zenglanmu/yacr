//! PLY (Polygon File Format) 1.0 reader.
//!
//! The header grammar, scalar types and both binary endiannesses are handled
//! here directly. Property order is **not** assumed: every declared property of
//! `element vertex` is read in the declared order and `x`/`y`/`z`, the optional
//! `nx`/`ny`/`nz` normals and the optional `red,green,blue` / `r,g,b` colours
//! are selected by name. Unknown properties (and unknown elements) are consumed
//! with their exact declared width, which is what keeps a file with extra
//! attributes from silently shifting every later value.
//!
//! Strictness is deliberate:
//! * Unknown header directives, formats, scalar type names and malformed
//!   `property` lines are `CorruptData`, never guessed.
//! * Every declared element instance must be present in the payload. Running out
//!   of bytes names the element, instance and missing piece
//!   ([`ERR_TRUNCATED`]).
//! * Trailing non-whitespace bytes after the declared elements are rejected
//!   ([`ERR_TRAILING_BYTES`]).
//! * `x`/`y`/`z` must be finite; a non-finite coordinate cannot be repaired
//!   without inventing geometry.
//! * Faces with `n > 3` are fan-triangulated and reported ([`DIAG_NGON_FAN`] +
//!   `Partial`); `n < 3` is reported and skipped; degenerate triangles are
//!   dropped and reported.

use cad_domain::{CadError, CadResult};
use cad_import_core::{
    compute_identity, ImportPhase, ImportProgress, ImportProgressSink, ImportRequest,
    ImportedDrawing, Importer, NoopProgress,
};

use crate::publish;

/// Stable diagnostic code: an n-gon was fan-triangulated.
pub const DIAG_NGON_FAN: &str = "import.ply.ngon_fan_triangulated";
/// Stable diagnostic code: a face had fewer than three corners and was skipped.
pub const DIAG_FACE_TOO_FEW_VERTICES: &str = "import.ply.face_too_few_vertices";
/// Stable diagnostic code: a degenerate triangle was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.ply.degenerate_triangle";
/// Stable diagnostic code: declared normals were unusable and dropped.
pub const DIAG_NORMALS_DROPPED: &str = "import.ply.normals_dropped";
/// Stable diagnostic code: declared colours were unusable and dropped.
pub const DIAG_COLORS_DROPPED: &str = "import.ply.colors_dropped";

/// Stable refusal code: the `format` line is not a known PLY 1.0 encoding.
pub const ERR_UNSUPPORTED_FORMAT: &str = "import.ply.unsupported_format";
/// Stable refusal code: a scalar/property type name is not a PLY 1.0 type.
pub const ERR_UNSUPPORTED_PROPERTY_TYPE: &str = "import.ply.unsupported_property_type";
/// Stable refusal code: the header grammar is unknown or malformed.
pub const ERR_MALFORMED_HEADER: &str = "import.ply.malformed_header";
/// Stable refusal code: the header has no `end_header` line.
pub const ERR_MISSING_END_HEADER: &str = "import.ply.missing_end_header";
/// Stable refusal code: the payload ends before the declared elements do.
pub const ERR_TRUNCATED: &str = "import.ply.truncated_payload";
/// Stable refusal code: non-whitespace bytes follow the declared elements.
pub const ERR_TRAILING_BYTES: &str = "import.ply.trailing_bytes";
/// Stable refusal code: a vertex coordinate is not finite.
pub const ERR_NON_FINITE: &str = "import.ply.non_finite_coordinate";
/// Stable refusal code: a face index is outside the vertex list.
pub const ERR_INDEX_RANGE: &str = "import.ply.face_index_out_of_range";

/// The PLY entity `type_key` reported in capabilities and stored rows.
pub const PLY_MESH_TYPE_KEY: &str = "PlyMesh";

/// The PLY importer.
#[derive(Default)]
pub struct PlyImporter;

impl PlyImporter {
    pub fn new() -> Self {
        Self
    }
}

impl Importer for PlyImporter {
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
                "PLY is one mesh entity, over the 0 entity import limit".into(),
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
            PLY_MESH_TYPE_KEY,
            "PLY",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Ascii,
    BinaryLittleEndian,
    BinaryBigEndian,
}

impl Encoding {
    fn is_binary(self) -> bool {
        matches!(
            self,
            Encoding::BinaryLittleEndian | Encoding::BinaryBigEndian
        )
    }

    fn little_endian(self) -> bool {
        self == Encoding::BinaryLittleEndian
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl ScalarType {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "char" | "int8" => ScalarType::I8,
            "uchar" | "uint8" => ScalarType::U8,
            "short" | "int16" => ScalarType::I16,
            "ushort" | "uint16" => ScalarType::U16,
            "int" | "int32" => ScalarType::I32,
            "uint" | "uint32" => ScalarType::U32,
            "float" | "float32" => ScalarType::F32,
            "double" | "float64" => ScalarType::F64,
            _ => return None,
        })
    }

    fn width(self) -> usize {
        match self {
            ScalarType::I8 | ScalarType::U8 => 1,
            ScalarType::I16 | ScalarType::U16 => 2,
            ScalarType::I32 | ScalarType::U32 | ScalarType::F32 => 4,
            ScalarType::F64 => 8,
        }
    }

    fn is_integer(self) -> bool {
        matches!(
            self,
            ScalarType::I8
                | ScalarType::U8
                | ScalarType::I16
                | ScalarType::U16
                | ScalarType::I32
                | ScalarType::U32
        )
    }

    fn is_float(self) -> bool {
        matches!(self, ScalarType::F32 | ScalarType::F64)
    }
}

#[derive(Debug, Clone)]
enum PropertyKind {
    Scalar(ScalarType),
    List { count: ScalarType, item: ScalarType },
}

#[derive(Debug, Clone)]
struct HeaderProperty {
    name: String,
    kind: PropertyKind,
}

#[derive(Debug, Clone)]
struct HeaderElement {
    name: String,
    count: usize,
    properties: Vec<HeaderProperty>,
}

#[derive(Debug)]
struct Header {
    encoding: Encoding,
    elements: Vec<HeaderElement>,
}

/// A cursor over header/payload lines. `pos` points just past the last returned
/// line (including its `\n`).
struct LineCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> LineCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn next_line(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let rest = &self.bytes[self.pos..];
        match rest.iter().position(|byte| *byte == b'\n') {
            Some(offset) => {
                let line = &rest[..offset];
                self.pos += offset + 1;
                Some(line)
            }
            None => {
                self.pos = self.bytes.len();
                Some(rest)
            }
        }
    }
}

fn header_text(line: &[u8], number: usize) -> CadResult<&str> {
    std::str::from_utf8(line).map_err(|_| {
        CadError::CorruptData(format!(
            "{ERR_MALFORMED_HEADER}: header line {number} is not valid UTF-8 text"
        ))
    })
}

fn malformed(message: impl std::fmt::Display) -> CadError {
    CadError::CorruptData(format!("{ERR_MALFORMED_HEADER}: {message}"))
}

/// Parse the header and return it with the payload's byte offset.
fn parse_header(bytes: &[u8]) -> CadResult<(Header, usize)> {
    let mut cursor = LineCursor::new(bytes);
    let first = cursor
        .next_line()
        .ok_or_else(|| malformed("empty file, expected the 'ply' magic line"))?;
    if header_text(first, 1)?.trim() != "ply" {
        return Err(malformed("file does not start with the 'ply' magic line"));
    }

    let mut encoding: Option<Encoding> = None;
    let mut elements: Vec<HeaderElement> = Vec::new();
    let mut ended = false;
    let mut line_number = 1usize;

    while let Some(line) = cursor.next_line() {
        line_number += 1;
        let text = header_text(line, line_number)?.trim();
        if text.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = text.split_whitespace().collect();
        match tokens[0] {
            "comment" | "obj_info" => {}
            "format" => {
                if tokens.len() != 3 {
                    return Err(malformed(format!(
                        "line {line_number}: 'format' expects '<encoding> 1.0'"
                    )));
                }
                if encoding.is_some() {
                    return Err(malformed(format!(
                        "line {line_number}: duplicate 'format' line"
                    )));
                }
                if tokens[2] != "1.0" {
                    return Err(CadError::CorruptData(format!(
                        "{ERR_UNSUPPORTED_FORMAT}: PLY version '{}' is not supported; only 1.0 is read",
                        tokens[2]
                    )));
                }
                encoding = Some(match tokens[1] {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::BinaryLittleEndian,
                    "binary_big_endian" => Encoding::BinaryBigEndian,
                    other => {
                        return Err(CadError::CorruptData(format!(
                        "{ERR_UNSUPPORTED_FORMAT}: 'format {other} 1.0' is not a PLY 1.0 encoding"
                    )))
                    }
                });
            }
            "element" => {
                if encoding.is_none() {
                    return Err(malformed(format!(
                        "line {line_number}: 'element' appears before 'format'"
                    )));
                }
                if tokens.len() != 3 {
                    return Err(malformed(format!(
                        "line {line_number}: 'element' expects '<name> <count>'"
                    )));
                }
                let name = tokens[1].to_string();
                if name.is_empty() {
                    return Err(malformed(format!(
                        "line {line_number}: element name is empty"
                    )));
                }
                if elements.iter().any(|element| element.name == name) {
                    return Err(malformed(format!(
                        "line {line_number}: duplicate element '{name}'"
                    )));
                }
                let count = tokens[2].parse::<usize>().map_err(|_| {
                    malformed(format!(
                        "line {line_number}: element count '{}' is not a non-negative integer",
                        tokens[2]
                    ))
                })?;
                elements.push(HeaderElement {
                    name,
                    count,
                    properties: Vec::new(),
                });
            }
            "property" => {
                let Some(element) = elements.last_mut() else {
                    return Err(malformed(format!(
                        "line {line_number}: 'property' appears before any 'element'"
                    )));
                };
                let (name, kind) = if tokens.len() == 5 && tokens[1] == "list" {
                    let count = ScalarType::from_name(tokens[2]).ok_or_else(|| {
                        CadError::CorruptData(format!(
                            "{ERR_UNSUPPORTED_PROPERTY_TYPE}: PLY list count type '{}' is unknown",
                            tokens[2]
                        ))
                    })?;
                    if !count.is_integer() {
                        return Err(CadError::CorruptData(format!(
                            "{ERR_UNSUPPORTED_PROPERTY_TYPE}: PLY list count type '{}' is not an integer",
                            tokens[2]
                        )));
                    }
                    let item = ScalarType::from_name(tokens[3]).ok_or_else(|| {
                        CadError::CorruptData(format!(
                            "{ERR_UNSUPPORTED_PROPERTY_TYPE}: PLY list item type '{}' is unknown",
                            tokens[3]
                        ))
                    })?;
                    (tokens[4].to_string(), PropertyKind::List { count, item })
                } else if tokens.len() == 3 {
                    let scalar = ScalarType::from_name(tokens[1]).ok_or_else(|| {
                        CadError::CorruptData(format!(
                            "{ERR_UNSUPPORTED_PROPERTY_TYPE}: PLY scalar type '{}' is unknown",
                            tokens[1]
                        ))
                    })?;
                    (tokens[2].to_string(), PropertyKind::Scalar(scalar))
                } else {
                    return Err(malformed(format!(
                        "line {line_number}: 'property' expects '<type> <name>' or 'list <count-type> <item-type> <name>'"
                    )));
                };
                if name.is_empty() {
                    return Err(malformed(format!(
                        "line {line_number}: property name is empty"
                    )));
                }
                if element
                    .properties
                    .iter()
                    .any(|property| property.name == name)
                {
                    return Err(malformed(format!(
                        "line {line_number}: duplicate property '{name}' on element '{}'",
                        element.name
                    )));
                }
                element.properties.push(HeaderProperty { name, kind });
            }
            "end_header" => {
                if tokens.len() != 1 {
                    return Err(malformed(format!(
                        "line {line_number}: 'end_header' takes no arguments"
                    )));
                }
                ended = true;
                break;
            }
            other => {
                return Err(malformed(format!(
                    "line {line_number}: unsupported header directive '{other}'"
                )))
            }
        }
    }

    if !ended {
        return Err(CadError::CorruptData(format!(
            "{ERR_MISSING_END_HEADER}: header ended before an 'end_header' line"
        )));
    }
    let encoding = encoding.ok_or_else(|| malformed("header has no 'format' line"))?;
    if elements.is_empty() {
        return Err(malformed("header declares no elements"));
    }
    Ok((Header { encoding, elements }, cursor.pos))
}

#[derive(Debug, Clone, Copy)]
enum ScalarValue {
    Int(i64),
    Float(f64),
}

impl ScalarValue {
    fn as_f64(self) -> f64 {
        match self {
            ScalarValue::Int(value) => value as f64,
            ScalarValue::Float(value) => value,
        }
    }

    fn as_index(self, property: &str) -> CadResult<u32> {
        match self {
            ScalarValue::Int(value) if (0..=u32::MAX as i64).contains(&value) => Ok(value as u32),
            ScalarValue::Int(value) => Err(CadError::CorruptData(format!(
                "{ERR_INDEX_RANGE}: property '{property}' holds {value}, outside the u32 index range"
            ))),
            ScalarValue::Float(value) => Err(CadError::CorruptData(format!(
                "{ERR_INDEX_RANGE}: property '{property}' holds the non-integer value {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
enum PropertyValue {
    Scalar(ScalarValue),
    List(Vec<ScalarValue>),
}

impl PropertyValue {
    fn scalar(self, element: &str, property: &str) -> CadResult<ScalarValue> {
        match self {
            PropertyValue::Scalar(value) => Ok(value),
            PropertyValue::List(_) => Err(CadError::CorruptData(format!(
                "{ERR_MALFORMED_HEADER}: element '{element}' property '{property}' is a list where a scalar is required"
            ))),
        }
    }

    fn list(self, element: &str, property: &str) -> CadResult<Vec<ScalarValue>> {
        match self {
            PropertyValue::List(values) => Ok(values),
            PropertyValue::Scalar(_) => Err(CadError::CorruptData(format!(
                "{ERR_MALFORMED_HEADER}: element '{element}' property '{property}' is a scalar where a list is required"
            ))),
        }
    }
}

struct AsciiPayload<'a> {
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<&'a str>,
    cursor: usize,
    line: usize,
}

struct BinaryPayload<'a> {
    bytes: &'a [u8],
    pos: usize,
    little_endian: bool,
}

enum Payload<'a> {
    Ascii(AsciiPayload<'a>),
    Binary(BinaryPayload<'a>),
}

impl<'a> Payload<'a> {
    fn new(bytes: &'a [u8], encoding: Encoding) -> Self {
        match encoding {
            Encoding::Ascii => Payload::Ascii(AsciiPayload {
                bytes,
                pos: 0,
                tokens: Vec::new(),
                cursor: 0,
                line: 0,
            }),
            _ => Payload::Binary(BinaryPayload {
                bytes,
                pos: 0,
                little_endian: encoding.little_endian(),
            }),
        }
    }

    fn begin_instance(&mut self, element: &str, instance: usize) -> CadResult<()> {
        let Payload::Ascii(ascii) = self else {
            return Ok(());
        };
        if ascii.pos >= ascii.bytes.len() {
            return Err(CadError::CorruptData(format!(
                "{ERR_TRUNCATED}: payload ended before element '{element}' instance {instance}"
            )));
        }
        let rest = &ascii.bytes[ascii.pos..];
        let (line, advance) = match rest.iter().position(|byte| *byte == b'\n') {
            Some(offset) => (&rest[..offset], offset + 1),
            None => (rest, rest.len()),
        };
        ascii.pos += advance;
        ascii.line += 1;
        let text = std::str::from_utf8(line).map_err(|_| {
            CadError::CorruptData(format!(
                "{ERR_MALFORMED_HEADER}: payload line {} is not valid UTF-8 text",
                ascii.line
            ))
        })?;
        ascii.tokens = text.split_whitespace().collect();
        ascii.cursor = 0;
        Ok(())
    }

    fn end_instance(&mut self, element: &str, instance: usize) -> CadResult<()> {
        let Payload::Ascii(ascii) = self else {
            return Ok(());
        };
        if ascii.cursor < ascii.tokens.len() {
            return Err(CadError::CorruptData(format!(
                "{ERR_MALFORMED_HEADER}: element '{element}' instance {instance} has {} unexpected trailing token(s)",
                ascii.tokens.len() - ascii.cursor
            )));
        }
        Ok(())
    }

    fn read_scalar(
        &mut self,
        ty: ScalarType,
        element: &str,
        instance: usize,
        property: &str,
    ) -> CadResult<ScalarValue> {
        match self {
            Payload::Ascii(ascii) => {
                let token = ascii.tokens.get(ascii.cursor).copied().ok_or_else(|| {
                    CadError::CorruptData(format!(
                        "{ERR_TRUNCATED}: element '{element}' instance {instance} is missing property '{property}'"
                    ))
                })?;
                ascii.cursor += 1;
                parse_ascii_scalar(ty, token).map_err(|()| {
                    CadError::CorruptData(format!(
                        "{ERR_MALFORMED_HEADER}: element '{element}' instance {instance} property '{property}' value '{token}' is not a valid {ty:?}"
                    ))
                })
            }
            Payload::Binary(binary) => {
                let width = ty.width();
                let remaining = binary.bytes.len().saturating_sub(binary.pos);
                if remaining < width {
                    return Err(CadError::CorruptData(format!(
                        "{ERR_TRUNCATED}: payload ended while reading element '{element}' instance {instance} property '{property}' (needed {width} byte(s), {remaining} remain)"
                    )));
                }
                let raw = &binary.bytes[binary.pos..binary.pos + width];
                binary.pos += width;
                Ok(read_binary_scalar(ty, raw, binary.little_endian))
            }
        }
    }

    fn read_list(
        &mut self,
        count_ty: ScalarType,
        item_ty: ScalarType,
        element: &str,
        instance: usize,
        property: &str,
        retain: bool,
    ) -> CadResult<Vec<ScalarValue>> {
        let count = match self.read_scalar(count_ty, element, instance, property)? {
            ScalarValue::Int(value) if value >= 0 => value as usize,
            ScalarValue::Int(value) => {
                return Err(CadError::CorruptData(format!(
                    "{ERR_MALFORMED_HEADER}: element '{element}' instance {instance} property '{property}' declares a negative list count {value}"
                )))
            }
            ScalarValue::Float(value) => {
                return Err(CadError::CorruptData(format!(
                    "{ERR_MALFORMED_HEADER}: element '{element}' instance {instance} property '{property}' has non-integer list count {value}"
                )))
            }
        };

        let mut values = if retain {
            Vec::with_capacity(count)
        } else {
            Vec::new()
        };
        match self {
            Payload::Ascii(ascii) => {
                let remaining = ascii.tokens.len().saturating_sub(ascii.cursor);
                if count > remaining {
                    return Err(CadError::CorruptData(format!(
                        "{ERR_TRUNCATED}: element '{element}' instance {instance} property '{property}' declares {count} item(s), only {remaining} token(s) remain on the line"
                    )));
                }
                for _ in 0..count {
                    let token = ascii.tokens[ascii.cursor];
                    ascii.cursor += 1;
                    let value = parse_ascii_scalar(item_ty, token).map_err(|()| {
                        CadError::CorruptData(format!(
                            "{ERR_MALFORMED_HEADER}: element '{element}' instance {instance} property '{property}' list item '{token}' is not a valid {item_ty:?}"
                        ))
                    })?;
                    if retain {
                        values.push(value);
                    }
                }
            }
            Payload::Binary(binary) => {
                let width = item_ty.width();
                let needed = count.saturating_mul(width);
                let remaining = binary.bytes.len().saturating_sub(binary.pos);
                if needed > remaining {
                    return Err(CadError::CorruptData(format!(
                        "{ERR_TRUNCATED}: element '{element}' instance {instance} property '{property}' declares {count} item(s) of {width} byte(s), only {remaining} byte(s) remain"
                    )));
                }
                for _ in 0..count {
                    let raw = &binary.bytes[binary.pos..binary.pos + width];
                    binary.pos += width;
                    if retain {
                        values.push(read_binary_scalar(item_ty, raw, binary.little_endian));
                    }
                }
            }
        }
        Ok(values)
    }

    fn finish(&self) -> CadResult<()> {
        let (bytes, pos) = match self {
            Payload::Ascii(ascii) => (ascii.bytes, ascii.pos),
            Payload::Binary(binary) => (binary.bytes, binary.pos),
        };
        let trailing = &bytes[pos..];
        let extra = trailing
            .iter()
            .filter(|byte| !matches!(**byte, b' ' | b'\t' | b'\r' | b'\n'))
            .count();
        if extra > 0 {
            return Err(CadError::CorruptData(format!(
                "{ERR_TRAILING_BYTES}: {extra} non-whitespace byte(s) follow the declared element payload"
            )));
        }
        Ok(())
    }
}

fn parse_ascii_scalar(ty: ScalarType, token: &str) -> Result<ScalarValue, ()> {
    Ok(match ty {
        ScalarType::I8 => ScalarValue::Int(token.parse::<i8>().map_err(|_| ())? as i64),
        ScalarType::U8 => ScalarValue::Int(token.parse::<u8>().map_err(|_| ())? as i64),
        ScalarType::I16 => ScalarValue::Int(token.parse::<i16>().map_err(|_| ())? as i64),
        ScalarType::U16 => ScalarValue::Int(token.parse::<u16>().map_err(|_| ())? as i64),
        ScalarType::I32 => ScalarValue::Int(token.parse::<i32>().map_err(|_| ())? as i64),
        ScalarType::U32 => ScalarValue::Int(token.parse::<u32>().map_err(|_| ())? as i64),
        ScalarType::F32 => ScalarValue::Float(token.parse::<f32>().map_err(|_| ())? as f64),
        ScalarType::F64 => ScalarValue::Float(token.parse::<f64>().map_err(|_| ())?),
    })
}

fn read_binary_scalar(ty: ScalarType, raw: &[u8], little_endian: bool) -> ScalarValue {
    macro_rules! read {
        ($integer:ty) => {{
            let mut bytes = [0u8; std::mem::size_of::<$integer>()];
            bytes.copy_from_slice(raw);
            if little_endian {
                <$integer>::from_le_bytes(bytes)
            } else {
                <$integer>::from_be_bytes(bytes)
            }
        }};
    }
    match ty {
        ScalarType::I8 => ScalarValue::Int(raw[0] as i8 as i64),
        ScalarType::U8 => ScalarValue::Int(raw[0] as i64),
        ScalarType::I16 => ScalarValue::Int(read!(i16) as i64),
        ScalarType::U16 => ScalarValue::Int(read!(u16) as i64),
        ScalarType::I32 => ScalarValue::Int(read!(i32) as i64),
        ScalarType::U32 => ScalarValue::Int(read!(u32) as i64),
        ScalarType::F32 => ScalarValue::Float(read!(f32) as f64),
        ScalarType::F64 => ScalarValue::Float(read!(f64)),
    }
}

fn min_instance_width(element: &HeaderElement) -> usize {
    element
        .properties
        .iter()
        .map(|property| match property.kind {
            PropertyKind::Scalar(ty) => ty.width(),
            // With an empty list the instance is only the count field.
            PropertyKind::List { count, .. } => count.width(),
        })
        .sum()
}

fn ensure_element_fits(
    encoding: Encoding,
    element: &HeaderElement,
    remaining: usize,
) -> CadResult<()> {
    let needed = if encoding.is_binary() {
        element
            .count
            .checked_mul(min_instance_width(element))
            .ok_or_else(|| {
                CadError::CorruptData(format!(
                    "{ERR_TRUNCATED}: element '{}' declares {} instances, which overflows the payload size",
                    element.name, element.count
                ))
            })?
    } else {
        // One ASCII instance occupies at least one byte (its line ending).
        element.count
    };
    if needed > remaining {
        return Err(CadError::CorruptData(format!(
            "{ERR_TRUNCATED}: element '{}' declares {} instance(s) needing at least {needed} byte(s), only {remaining} payload byte(s) remain",
            element.name, element.count
        )));
    }
    Ok(())
}

fn read_instance(
    payload: &mut Payload<'_>,
    element: &HeaderElement,
    instance: usize,
    retain: bool,
    retain_list: Option<usize>,
) -> CadResult<Vec<PropertyValue>> {
    payload.begin_instance(&element.name, instance)?;
    let mut values = Vec::with_capacity(if retain { element.properties.len() } else { 0 });
    for (index, property) in element.properties.iter().enumerate() {
        match property.kind {
            PropertyKind::Scalar(ty) => {
                let value = payload.read_scalar(ty, &element.name, instance, &property.name)?;
                if retain {
                    values.push(PropertyValue::Scalar(value));
                }
            }
            PropertyKind::List { count, item } => {
                let keep = retain_list == Some(index);
                let items = payload.read_list(
                    count,
                    item,
                    &element.name,
                    instance,
                    &property.name,
                    keep,
                )?;
                if retain {
                    values.push(PropertyValue::List(items));
                }
            }
        }
    }
    payload.end_instance(&element.name, instance)?;
    Ok(values)
}

mod parse;
pub(crate) use parse::parse;
