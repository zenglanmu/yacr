//! PLY element/payload orchestration and mesh assembly.
//!
//! This is a child of [`super`] so it can use the reader's private header
//! and payload types without widening their visibility.

use super::*;
use crate::{Note, ParsedMesh};
use cad_domain::Point3;

fn scalar_property(element: &HeaderElement, name: &str) -> Option<(usize, ScalarType)> {
    element
        .properties
        .iter()
        .position(|property| property.name == name)
        .and_then(|index| match element.properties[index].kind {
            PropertyKind::Scalar(ty) => Some((index, ty)),
            PropertyKind::List { .. } => None,
        })
}

struct ColorLayout {
    red: usize,
    green: usize,
    blue: usize,
    ty: ScalarType,
}

fn color_layout(element: &HeaderElement) -> Result<Option<ColorLayout>, String> {
    let mut partial = false;
    for names in [["red", "green", "blue"], ["r", "g", "b"]] {
        let found = names.map(|name| {
            element
                .properties
                .iter()
                .position(|property| property.name == name)
        });
        if !found.iter().all(Option::is_some) {
            if found.iter().any(Option::is_some) {
                partial = true;
            }
            continue;
        }
        let mut indices = [0usize; 3];
        let mut types = [ScalarType::U8; 3];
        for (slot, index) in found.iter().enumerate() {
            let index = index.expect("all three names were found");
            indices[slot] = index;
            match element.properties[index].kind {
                PropertyKind::Scalar(ty) => types[slot] = ty,
                PropertyKind::List { .. } => {
                    return Err(format!(
                        "colour property '{}' is a list, not a scalar",
                        names[slot]
                    ))
                }
            }
        }
        let [red_ty, green_ty, blue_ty] = types;
        if red_ty != green_ty || green_ty != blue_ty {
            return Err(format!(
                "colour properties '{}', '{}' and '{}' use different scalar types",
                names[0], names[1], names[2]
            ));
        }
        if red_ty != ScalarType::U8 && !red_ty.is_float() {
            return Err(format!(
                "colour properties use {}; only uchar 0-255 or float 0-1 are mapped",
                type_label(red_ty)
            ));
        }
        return Ok(Some(ColorLayout {
            red: indices[0],
            green: indices[1],
            blue: indices[2],
            ty: red_ty,
        }));
    }
    if partial {
        return Err("only part of a colour property set was declared".to_string());
    }
    Ok(None)
}

fn type_label(ty: ScalarType) -> &'static str {
    match ty {
        ScalarType::I8 => "char/int8",
        ScalarType::U8 => "uchar/uint8",
        ScalarType::I16 => "short/int16",
        ScalarType::U16 => "ushort/uint16",
        ScalarType::I32 => "int/int32",
        ScalarType::U32 => "uint/uint32",
        ScalarType::F32 => "float/float32",
        ScalarType::F64 => "double/float64",
    }
}

pub(crate) fn parse(bytes: &[u8], cancelled: &dyn Fn() -> bool) -> CadResult<ParsedMesh> {
    let (header, payload_start) = parse_header(bytes)?;
    let payload_bytes = &bytes[payload_start..];
    let mut payload = Payload::new(payload_bytes, header.encoding);

    let mut vertices: Option<Vec<Point3>> = None;
    let mut normals: Option<Vec<Point3>> = None;
    let mut colors: Option<Vec<[u8; 3]>> = None;
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut ngon_faces = 0usize;
    let mut short_faces = 0usize;
    let mut repeated_index_triangles = 0usize;
    let mut normals_note: Option<String> = None;
    let mut colors_note: Option<String> = None;
    let mut saw_vertex = false;

    for element in &header.elements {
        // A binary element with no properties consumes no bytes, so its declared
        // instance count must not drive a pointless loop. ASCII still consumes
        // one line per instance; `ensure_element_fits` bounds that by the file
        // size.
        if element.properties.is_empty() && header.encoding.is_binary() {
            continue;
        }
        if element.name == "vertex" {
            if saw_vertex {
                return Err(malformed("more than one 'vertex' element is not supported"));
            }
            saw_vertex = true;
            if element.count > u32::MAX as usize {
                return Err(CadError::CorruptData(format!(
                    "{ERR_MALFORMED_HEADER}: vertex element declares {} vertices, more than the u32 mesh index range",
                    element.count
                )));
            }
            let (x, _) = scalar_property(element, "x")
                .ok_or_else(|| malformed("vertex element has no scalar 'x' property"))?;
            let (y, _) = scalar_property(element, "y")
                .ok_or_else(|| malformed("vertex element has no scalar 'y' property"))?;
            let (z, _) = scalar_property(element, "z")
                .ok_or_else(|| malformed("vertex element has no scalar 'z' property"))?;
            let has_normal_name = ["nx", "ny", "nz"].iter().any(|name| {
                element
                    .properties
                    .iter()
                    .any(|property| property.name == *name)
            });
            let normal_layout = match (
                scalar_property(element, "nx"),
                scalar_property(element, "ny"),
                scalar_property(element, "nz"),
            ) {
                (Some((nx, _)), Some((ny, _)), Some((nz, _))) => Some((nx, ny, nz)),
                (None, None, None) if !has_normal_name => None,
                _ => {
                    normals_note = Some(
                        "nx/ny/nz were only partially declared or were not scalars; vertex normals were dropped"
                            .to_string(),
                    );
                    None
                }
            };
            let color_layout = match color_layout(element) {
                Ok(layout) => layout,
                Err(reason) => {
                    colors_note = Some(format!("{reason}; vertex colours were dropped"));
                    None
                }
            };

            ensure_element_fits(header.encoding, element, payload_bytes.len())?;
            let mut vertex_values = Vec::with_capacity(element.count);
            let mut normal_values = Vec::with_capacity(element.count);
            let mut color_values = Vec::with_capacity(element.count);
            let mut normals_failed = normal_layout.is_none();
            let mut colors_failed = color_layout.is_none();

            for instance in 0..element.count {
                if cancelled() {
                    return Err(CadError::Cancelled);
                }
                let values = read_instance(&mut payload, element, instance, true, None)?;
                let point = Point3 {
                    x: values[x].clone().scalar("vertex", "x")?.as_f64(),
                    y: values[y].clone().scalar("vertex", "y")?.as_f64(),
                    z: values[z].clone().scalar("vertex", "z")?.as_f64(),
                };
                if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                    return Err(CadError::CorruptData(format!(
                        "{ERR_NON_FINITE}: vertex {instance} has a non-finite coordinate"
                    )));
                }
                vertex_values.push(point);

                if let Some(layout) = &normal_layout {
                    let normal = Point3 {
                        x: values[layout.0].clone().scalar("vertex", "nx")?.as_f64(),
                        y: values[layout.1].clone().scalar("vertex", "ny")?.as_f64(),
                        z: values[layout.2].clone().scalar("vertex", "nz")?.as_f64(),
                    };
                    if !normal.x.is_finite() || !normal.y.is_finite() || !normal.z.is_finite() {
                        normals_failed = true;
                        normals_note = Some(
                            "a declared vertex normal was non-finite; normals were dropped"
                                .to_string(),
                        );
                    } else if !normals_failed {
                        normal_values.push(normal);
                    }
                }

                if let Some(layout) = &color_layout {
                    match vertex_color(&values, layout) {
                        Some(color) if !colors_failed => color_values.push(color),
                        Some(_) => {}
                        None => {
                            colors_failed = true;
                            colors_note = Some(
                                "a declared colour channel was outside its type's range; vertex colours were dropped"
                                    .to_string(),
                            );
                        }
                    }
                }
            }

            // A mid-stream failure discards the whole attribute list: a short
            // list would be indexed out of bounds by consumers.
            vertices = Some(vertex_values);
            normals = if normal_layout.is_some()
                && !normals_failed
                && normal_values.len() == element.count
            {
                Some(normal_values)
            } else {
                None
            };
            colors = if color_layout.is_some()
                && !colors_failed
                && color_values.len() == element.count
            {
                Some(color_values)
            } else {
                None
            };
        } else if element.name == "face" {
            let face_property = element
                .properties
                .iter()
                .position(|property| {
                    matches!(property.name.as_str(), "vertex_indices" | "vertex_index")
                        && matches!(property.kind, PropertyKind::List { .. })
                })
                .ok_or_else(|| {
                    malformed(
                        "face element has no 'property list <count> <item> vertex_indices' or 'vertex_index'",
                    )
                })?;
            let face_name = element.properties[face_property].name.clone();

            ensure_element_fits(header.encoding, element, payload_bytes.len())?;
            for instance in 0..element.count {
                if cancelled() {
                    return Err(CadError::Cancelled);
                }
                let values =
                    read_instance(&mut payload, element, instance, true, Some(face_property))?;
                let corners = values[face_property]
                    .clone()
                    .list("face", &face_name)?
                    .into_iter()
                    .map(|value| value.as_index(&face_name))
                    .collect::<CadResult<Vec<u32>>>()?;
                match corners.len() {
                    0..=2 => short_faces += 1,
                    3 => push_triangle(
                        [corners[0], corners[1], corners[2]],
                        &mut triangles,
                        &mut repeated_index_triangles,
                    ),
                    count => {
                        ngon_faces += 1;
                        for offset in 1..count - 1 {
                            push_triangle(
                                [corners[0], corners[offset], corners[offset + 1]],
                                &mut triangles,
                                &mut repeated_index_triangles,
                            );
                        }
                    }
                }
            }
        } else {
            ensure_element_fits(header.encoding, element, payload_bytes.len())?;
            for instance in 0..element.count {
                if cancelled() {
                    return Err(CadError::Cancelled);
                }
                read_instance(&mut payload, element, instance, false, None)?;
            }
        }
    }

    if !saw_vertex {
        return Err(malformed("file has no 'vertex' element"));
    }
    payload.finish()?;

    let vertices = vertices.unwrap_or_default();
    let vertex_count = vertices.len() as u32;
    for triangle in &triangles {
        if triangle.iter().any(|index| *index >= vertex_count) {
            return Err(CadError::CorruptData(format!(
                "{ERR_INDEX_RANGE}: face references a vertex outside the {vertex_count}-vertex list"
            )));
        }
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
    if let Some(message) = normals_note {
        notes.push(Note {
            code: DIAG_NORMALS_DROPPED,
            message,
            partial: false,
        });
    }
    if let Some(message) = colors_note {
        notes.push(Note {
            code: DIAG_COLORS_DROPPED,
            message,
            partial: false,
        });
    }
    Ok(ParsedMesh {
        vertices,
        triangles: kept,
        normals: normals.unwrap_or_default(),
        colors: colors.unwrap_or_default(),
        notes,
    })
}

/// Push a triangle unless it repeats an index. Zero-area filtering happens later,
/// once every vertex is known (PLY permits elements in any order).
fn push_triangle(triangle: [u32; 3], triangles: &mut Vec<[u32; 3]>, repeated: &mut usize) {
    if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
        *repeated += 1;
        return;
    }
    triangles.push(triangle);
}

fn vertex_color(values: &[PropertyValue], layout: &ColorLayout) -> Option<[u8; 3]> {
    let channel = |index: usize| -> Option<u8> {
        let value = values[index]
            .clone()
            .scalar("vertex", "colour")
            .ok()?
            .as_f64();
        match layout.ty {
            ScalarType::U8 => {
                if (0.0..=255.0).contains(&value) {
                    Some(value as u8)
                } else {
                    None
                }
            }
            ScalarType::F32 | ScalarType::F64 => {
                if value.is_finite() && (0.0..=1.0).contains(&value) {
                    Some((value * 255.0).round() as u8)
                } else {
                    None
                }
            }
            _ => None,
        }
    };
    Some([
        channel(layout.red)?,
        channel(layout.green)?,
        channel(layout.blue)?,
    ])
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
