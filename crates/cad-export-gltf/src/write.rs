//! The glTF 2.0 / GLB container writer.
//!
//! The JSON document is built by hand with `serde_json` instead of through
//! `gltf-json` constructors: the output is a closed, deterministic shell around
//! one tightly-packed binary buffer, and an explicit object graph makes the
//! byte layout auditable. There is exactly one buffer, no textures, no material
//! images and no external file references; `materials[]` exists only for an
//! explicit entity colour and carries a `baseColorFactor` alone.
//!
//! glTF colours are **linear**: stored sRGB vertex colours are decoded once into
//! `COLOR_0` `VEC4` floats, and an explicit entity colour becomes a linear
//! `baseColorFactor` with the entity opacity as alpha.
//!
//! Frame: `cad-db` is +Z up in source units; glTF is **metres, +Y up**. The
//! conversion `(x, y, z) -> (x * s, z * s, -y * s)` is applied once here, at the
//! boundary. Normals get the same rotation without the scale.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use cad_db::{EntityColor, EntityTransparency};
use cad_domain::{CadError, CadResult, Point3, Unit, UnitContext};
use serde_json::{json, Value};

use std::collections::BTreeMap;

use crate::collect::{Collected, SourceMesh};
use crate::report::ReportBuilder;
use crate::GltfContainer;

/// The exporter was handed a registry key it does not write.
pub(crate) const MISMATCHED_DESCRIPTOR: &str = "export.gltf.mismatched_descriptor";
/// The job carries no usable source unit, so a metre scale cannot be derived.
pub(crate) const UNITS_UNKNOWN: &str = "export.gltf.units_unknown";
/// A source geometry has no glTF mesh representation.
pub(crate) const UNSUPPORTED_GEOMETRY: &str = "export.gltf.unsupported_geometry";
/// A triangle is degenerate (zero area or repeated/out-of-range indices).
pub(crate) const DEGENERATE_TRIANGLE: &str = "export.gltf.degenerate_triangle";
/// The document has no writable mesh at all.
pub(crate) const NO_GEOMETRY: &str = "export.gltf.no_geometry";
/// Per-vertex normals were dropped (the document is only partially normalised).
pub(crate) const NORMALS_DROPPED: &str = "export.gltf.normals_dropped";
/// Per-vertex colours were dropped (the document is only partially coloured).
pub(crate) const COLORS_DROPPED: &str = "export.gltf.colors_dropped";
/// Provenance note: coordinates were scaled from the source unit to metres.
pub(crate) const UNITS_TO_METRES: &str = "export.gltf.units_to_metres";
/// Provenance note: the CAD +Z-up frame was rotated to glTF +Y up.
pub(crate) const FRAME_TO_Y_UP: &str = "export.gltf.frame_to_y_up";
/// Provenance note: layer-driven colour/opacity (`ByLayer`/`ByBlock`) has no
/// glTF representation, because glTF has no layers.
pub(crate) const LAYER_APPEARANCE: &str = "export.gltf.layer_appearance_not_written";
/// Per-vertex colours won a primitive, so its explicit entity colour was not
/// written as a material (the vertex attribute already carries the display
/// colour, and applying both would double the colour).
pub(crate) const ENTITY_COLOR_OVERRIDDEN: &str = "export.gltf.entity_color_overridden";
/// Per-vertex colours carry the display colour, but `COLOR_0` alpha is written
/// opaque, so an explicit entity opacity below `1.0` is not stored.
pub(crate) const VERTEX_COLOR_OPACITY: &str = "export.gltf.vertex_color_opacity_not_written";

/// `ARRAY_BUFFER` bufferView target.
const ARRAY_BUFFER: u32 = 34962;
/// `ELEMENT_ARRAY_BUFFER` bufferView target.
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
/// `FLOAT` accessor component type.
const FLOAT: u32 = 5126;
/// `UNSIGNED_LONG` (32-bit) accessor component type.
const UNSIGNED_INT: u32 = 5125;
/// glTF triangle-list primitive mode.
const TRIANGLES: u32 = 4;
/// GLB JSON chunk type (`JSON` as little-endian ASCII).
const CHUNK_JSON: u32 = 0x4E4F_534A;
/// GLB BIN chunk type (`BIN\0`).
const CHUNK_BIN: u32 = 0x004E_4942;
/// Prefix of the embedded `.gltf` buffer URI.
const DATA_URI_PREFIX: &str = "data:application/octet-stream;base64,";

/// The declared source-unit scale: metres per one `cad-db` coordinate unit.
///
/// glTF positions are metres, so the scale is mandatory. `Unit::DrawingUnits`,
/// a missing context or a non-finite factor is refused: assuming millimetres or
/// metres would silently rescale the model.
pub(crate) fn metres_per_source_unit(units: Option<&UnitContext>) -> CadResult<f64> {
    let units = units.ok_or_else(|| {
        CadError::Unsupported(format!(
            "{UNITS_UNKNOWN}: the job has no unit context; glTF positions are metres and the \
             source scale cannot be assumed"
        ))
    })?;
    let scale: f64 = match units.source {
        Unit::Millimeter => 0.001,
        Unit::Centimeter => 0.01,
        Unit::Meter => 1.0,
        Unit::Inch => 0.0254,
        Unit::Foot => 0.3048,
        Unit::DrawingUnits => {
            return Err(CadError::Unsupported(format!(
                "{UNITS_UNKNOWN}: source unit is DrawingUnits; the document does not declare a \
                 length"
            )))
        }
    };
    if !scale.is_finite() || scale <= 0.0 {
        return Err(CadError::Unsupported(format!(
            "{UNITS_UNKNOWN}: source unit scale {scale} is not a finite positive ratio"
        )));
    }
    Ok(scale)
}

/// The written container plus the part of the verdict only the writer knows.
pub(crate) struct Written {
    pub bytes: Vec<u8>,
    /// Source meshes written without losing a single triangle.
    pub exact_meshes: usize,
}

/// Write `collected` into `container`, recording every conversion and drop.
pub(crate) fn write(
    collected: &Collected,
    metres_per_unit: f64,
    container: GltfContainer,
    report: &mut ReportBuilder,
) -> CadResult<Written> {
    // Attribute policy is global: a partially present set is dropped everywhere
    // and reported once, rather than inventing values for the missing vertices.
    let include_normals = collected.normals_complete();
    let include_colors = collected.colors_complete();
    if collected.has_any_normals() && !include_normals {
        report.converted(
            NORMALS_DROPPED,
            "not every vertex has a finite normal; NORMAL was omitted from every primitive so \
             the file stays self-consistent"
                .to_string(),
        );
    }
    if collected.has_any_colors() && !include_colors {
        report.converted(
            COLORS_DROPPED,
            "not every vertex has a colour; COLOR_0 was omitted from every primitive so the file \
             stays self-consistent"
                .to_string(),
        );
    }

    let mut builder = Builder::default();
    let mut materials = Materials::default();
    let mut meshes_json = Vec::new();
    let mut nodes_json = Vec::new();
    let mut exact_meshes = 0usize;
    // glTF has no layers, so a layer-driven appearance is one documented gap no
    // matter how many entities inherit it.
    let mut layer_appearance = false;

    for mesh in &collected.meshes {
        let positions = positions(mesh, metres_per_unit);
        if positions
            .iter()
            .any(|position| position.iter().any(|value| !value.is_finite()))
        {
            // A coordinate outside the f32 range cannot be stored honestly; the
            // entity is refused by name instead of being written as infinity.
            report.lost_entity(crate::collect::DropEntry {
                object: mesh.object,
                type_key: mesh.type_key.clone(),
                code: UNSUPPORTED_GEOMETRY.to_string(),
                message: "a vertex is outside the f32 range glTF can store".to_string(),
            });
            continue;
        }

        let (triangles, degenerate) = writable_triangles(mesh);
        if degenerate > 0 {
            report.lost_triangles(
                mesh.object,
                &mesh.type_key,
                degenerate,
                DEGENERATE_TRIANGLE,
                format!(
                    "{degenerate} degenerate glTF triangle(s) were dropped (zero area or invalid \
                     vertex index); the source triangle cannot be written"
                ),
            );
        }
        if triangles.is_empty() {
            // Every triangle was degenerate; an empty primitive is invalid glTF
            // and the dropped triangles are already reported.
            continue;
        }

        let mut attributes = Vec::new();
        let position_accessor = builder.push_positions(&positions);
        attributes.push(("POSITION".to_string(), position_accessor));
        if include_normals {
            let normals = normals(mesh);
            attributes.push(("NORMAL".to_string(), builder.push_normals(&normals)));
        }
        let mut material_index = None;
        if include_colors {
            // `colors_complete` is a whole-document predicate, so a mesh written
            // with `COLOR_0` carries a colour for every vertex.
            attributes.push(("COLOR_0".to_string(), builder.push_colors(&mesh.colors)));
            if matches!(mesh.color, EntityColor::Explicit(_)) {
                // Vertex colours are the display colour; a material factor would
                // multiply on top of them and darken the surface.
                report.converted(
                    ENTITY_COLOR_OVERRIDDEN,
                    format!(
                        "entity #{} has an explicit colour but also per-vertex colours; \
                         COLOR_0 was written and the entity colour was not, so the vertex \
                         colours stay authoritative",
                        mesh.object.0
                    ),
                );
            }
            // `COLOR_0` alpha is written opaque, so an explicit opacity below
            // `1.0` cannot be carried by the vertex attribute and is reported
            // rather than silently dropped. A non-finite value is equally
            // unusable, so it takes the same conversion entry. `ByLayer`/
            // `ByBlock` opacity has no glTF equivalent and stays a layer note.
            match mesh.transparency {
                EntityTransparency::Explicit(alpha) if alpha.is_finite() && alpha >= 1.0 => {}
                EntityTransparency::Explicit(alpha) => {
                    report.converted(
                        VERTEX_COLOR_OPACITY,
                        format!(
                            "entity #{} has an explicit opacity of {alpha} but also per-vertex \
                             colours; COLOR_0 was written opaque, so the entity opacity was not \
                             stored",
                            mesh.object.0
                        ),
                    );
                }
                EntityTransparency::ByLayer | EntityTransparency::ByBlock => {
                    layer_appearance = true;
                }
            }
        } else {
            match mesh.color {
                EntityColor::Explicit(rgb) => {
                    let (alpha, from_layer) = opacity(mesh.transparency);
                    layer_appearance |= from_layer;
                    material_index = Some(materials.index_for(rgb, alpha));
                }
                EntityColor::ByLayer | EntityColor::ByBlock => layer_appearance = true,
            }
        }
        let index_accessor = builder.push_indices(&triangles);
        let mut primitive = json!({
            "attributes": attributes
                .into_iter()
                .map(|(name, accessor)| (name, Value::from(accessor)))
                .collect::<serde_json::Map<String, Value>>(),
            "indices": index_accessor,
            "mode": TRIANGLES,
        });
        if let Some(material) = material_index {
            primitive["material"] = Value::from(material);
        }
        let mesh_index = meshes_json.len();
        meshes_json.push(json!({
            "name": mesh.name,
            "primitives": [primitive],
        }));
        nodes_json.push(json!({
            "name": mesh.name,
            "mesh": mesh_index,
        }));
        if degenerate == 0 {
            exact_meshes += 1;
        }
    }

    if meshes_json.is_empty() {
        return Err(CadError::Unsupported(format!(
            "{NO_GEOMETRY}: no source mesh survived geometry filtering, so no file was written"
        )));
    }

    if layer_appearance {
        report.note(
            LAYER_APPEARANCE,
            "at least one entity takes its colour or opacity from its layer (ByLayer/ByBlock); \
             glTF has no layers, so the resolved appearance is not written"
                .to_string(),
        );
    }

    let (root, bin) = builder.into_document(meshes_json, nodes_json, materials.json, container);
    let json_bytes = serde_json::to_vec(&root)
        .map_err(|error| CadError::Invariant(format!("export.gltf.json: {error}")))?;
    let bytes = match container {
        GltfContainer::Gltf => json_bytes,
        GltfContainer::Glb => assemble_glb(json_bytes, &bin)?,
    };
    Ok(Written {
        bytes,
        exact_meshes,
    })
}

/// The scaled, frame-rotated positions in metres / glTF +Y up.
fn positions(mesh: &SourceMesh, metres_per_unit: f64) -> Vec<[f32; 3]> {
    mesh.vertices
        .iter()
        .map(|vertex| frame(vertex, metres_per_unit))
        .collect()
}

/// Rotate one CAD point into the glTF frame and scale it to metres.
fn frame(point: &Point3, metres_per_unit: f64) -> [f32; 3] {
    [
        (point.x * metres_per_unit) as f32,
        (point.z * metres_per_unit) as f32,
        (-(point.y * metres_per_unit)) as f32,
    ]
}

/// Rotate one CAD normal into the glTF frame (no scale).
fn frame_normal(normal: &Point3) -> [f32; 3] {
    [normal.x as f32, normal.z as f32, (-normal.y) as f32]
}

fn normals(mesh: &SourceMesh) -> Vec<[f32; 3]> {
    mesh.normals.iter().map(frame_normal).collect()
}

/// sRGB byte -> linear glTF component (the IEC 61966-2-1 decode).
///
/// Exact inverse of `cad_import_gltf::scene`'s `linear_to_srgb_byte`, so an
/// export/import round trip returns the stored byte (± rounding). Both halves
/// of the piecewise curve are used: the linear segment below the 0.0031308
/// breakpoint and the 2.4-power segment above it.
fn srgb_byte_to_linear(byte: u8) -> f32 {
    let encoded = f64::from(byte) / 255.0;
    let linear = if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    };
    linear as f32
}

/// The written material alpha and whether it had to fall back to opaque
/// because the opacity is inherited from a layer (or is unusable).
fn opacity(transparency: EntityTransparency) -> (f32, bool) {
    match transparency {
        EntityTransparency::Explicit(alpha) if alpha.is_finite() => (alpha.clamp(0.0, 1.0), false),
        EntityTransparency::Explicit(_) => (1.0, true),
        EntityTransparency::ByLayer | EntityTransparency::ByBlock => (1.0, true),
    }
}

/// Deduplicated `materials[]` entries for explicit entity colours.
///
/// Two entities that resolve to the same sRGB byte triple and opacity share one
/// material, so a drawing that colours thousands of entities from a handful of
/// colours produces a handful of materials.
#[derive(Default)]
struct Materials {
    json: Vec<Value>,
    indices: BTreeMap<(u8, u8, u8, u32), usize>,
}

impl Materials {
    /// The index of the material for `rgb` at `alpha`, adding it if needed.
    fn index_for(&mut self, rgb: [u8; 3], alpha: f32) -> usize {
        let key = (rgb[0], rgb[1], rgb[2], alpha.to_bits());
        if let Some(index) = self.indices.get(&key) {
            return *index;
        }
        let factor = [
            srgb_byte_to_linear(rgb[0]),
            srgb_byte_to_linear(rgb[1]),
            srgb_byte_to_linear(rgb[2]),
            alpha,
        ];
        let factor: Vec<Value> = factor
            .into_iter()
            .map(|value| Value::from(f64::from(value)))
            .collect();
        let mut material = json!({
            "name": format!("entity_color_{}_{}_{}", rgb[0], rgb[1], rgb[2]),
            "pbrMetallicRoughness": { "baseColorFactor": factor },
        });
        if alpha < 1.0 {
            // The glTF default is OPAQUE; BLEND is only needed when the stored
            // opacity actually lets background through.
            material["alphaMode"] = Value::from("BLEND");
        }
        self.json.push(material);
        let index = self.json.len() - 1;
        self.indices.insert(key, index);
        index
    }
}

/// The triangles that can be written plus the number of degenerate ones.
///
/// A triangle is degenerate when it repeats an index, names a vertex that does
/// not exist, or has zero (or non-finite) area after the source transform.
fn writable_triangles(mesh: &SourceMesh) -> (Vec<[u32; 3]>, usize) {
    let mut writable = Vec::with_capacity(mesh.triangles.len());
    let mut degenerate = 0usize;
    for triangle in &mesh.triangles {
        let points = (
            mesh.vertices.get(triangle[0] as usize),
            mesh.vertices.get(triangle[1] as usize),
            mesh.vertices.get(triangle[2] as usize),
        );
        let (Some(a), Some(b), Some(c)) = points else {
            degenerate += 1;
            continue;
        };
        if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
            degenerate += 1;
            continue;
        }
        let u = [b.x - a.x, b.y - a.y, b.z - a.z];
        let v = [c.x - a.x, c.y - a.y, c.z - a.z];
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length_squared = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
        if !length_squared.is_finite() || length_squared == 0.0 {
            degenerate += 1;
            continue;
        }
        writable.push(*triangle);
    }
    (writable, degenerate)
}

/// One tightly packed buffer, its bufferViews and its accessors.
#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    /// Append `bytes` (4-byte aligned) and return the new bufferView index.
    fn push_view(&mut self, bytes: &[u8], target: u32) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        self.views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": bytes.len(),
            "target": target,
        }));
        self.views.len() - 1
    }

    fn push_accessor(&mut self, accessor: Value) -> usize {
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    /// Positions: f32 `VEC3` with the mandatory `min`/`max`.
    fn push_positions(&mut self, positions: &[[f32; 3]]) -> usize {
        let mut bytes = Vec::with_capacity(positions.len() * 12);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for position in positions {
            for (axis, value) in position.iter().enumerate() {
                bytes.extend_from_slice(&value.to_le_bytes());
                min[axis] = min[axis].min(*value);
                max[axis] = max[axis].max(*value);
            }
        }
        let view = self.push_view(&bytes, ARRAY_BUFFER);
        let component = |values: [f32; 3]| -> Vec<Value> {
            values
                .into_iter()
                .map(|value| Value::from(f64::from(value)))
                .collect()
        };
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": positions.len(),
            "type": "VEC3",
            "min": component(min),
            "max": component(max),
        }))
    }

    fn push_normals(&mut self, normals: &[[f32; 3]]) -> usize {
        let mut bytes = Vec::with_capacity(normals.len() * 12);
        for normal in normals {
            for value in normal {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let view = self.push_view(&bytes, ARRAY_BUFFER);
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": normals.len(),
            "type": "VEC3",
        }))
    }

    /// Colours: `VEC4` `FLOAT`, linear RGB and an opaque alpha.
    ///
    /// glTF 2.0 defines `COLOR_0` values as **linear**; `cad-db` stores sRGB
    /// bytes, so each channel is decoded exactly once here instead of being
    /// written raw (which would display a gamma-shifted colour and would not
    /// round-trip through the importer).
    fn push_colors(&mut self, colors: &[[u8; 3]]) -> usize {
        let mut bytes = Vec::with_capacity(colors.len() * 16);
        for color in colors {
            for channel in color {
                bytes.extend_from_slice(&srgb_byte_to_linear(*channel).to_le_bytes());
            }
            bytes.extend_from_slice(&1.0f32.to_le_bytes());
        }
        let view = self.push_view(&bytes, ARRAY_BUFFER);
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": colors.len(),
            "type": "VEC4",
        }))
    }

    fn push_indices(&mut self, triangles: &[[u32; 3]]) -> usize {
        let mut bytes = Vec::with_capacity(triangles.len() * 12);
        for triangle in triangles {
            for index in triangle {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
        }
        let view = self.push_view(&bytes, ELEMENT_ARRAY_BUFFER);
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": UNSIGNED_INT,
            "count": triangles.len() * 3,
            "type": "SCALAR",
        }))
    }

    /// Finish the document shell and hand back its binary payload. The bin
    /// buffer is padded to 4 bytes and its `byteLength` covers that padding, so
    /// the GLB BIN chunk length matches it exactly.
    fn into_document(
        mut self,
        meshes: Vec<Value>,
        nodes: Vec<Value>,
        materials: Vec<Value>,
        container: GltfContainer,
    ) -> (Value, Vec<u8>) {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let buffer = match container {
            GltfContainer::Gltf => json!({
                "byteLength": self.bin.len(),
                "uri": format!("{DATA_URI_PREFIX}{}", STANDARD.encode(&self.bin)),
            }),
            GltfContainer::Glb => json!({ "byteLength": self.bin.len() }),
        };
        let mut root = json!({
            "asset": { "version": "2.0", "generator": "yacr" },
            "scene": 0,
            "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<usize>>() }],
            "nodes": nodes,
            "meshes": meshes,
            "buffers": [buffer],
            "bufferViews": self.views,
            "accessors": self.accessors,
        });
        if !materials.is_empty() {
            // Omitted when no entity had an explicit colour: an empty array
            // would be valid but would claim a material list that does not exist.
            root["materials"] = Value::Array(materials);
        }
        (root, self.bin)
    }
}

/// Wrap the JSON document into the binary glTF container.
fn assemble_glb(mut json_bytes: Vec<u8>, bin: &[u8]) -> CadResult<Vec<u8>> {
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let total = u32::try_from(total).map_err(|_| {
        CadError::InvalidInput("export.gltf.too_large: GLB container exceeds 4 GiB".to_string())
    })?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    out.extend_from_slice(bin);
    Ok(out)
}
