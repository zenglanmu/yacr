//! glTF 2.0 (`.gltf`) and binary glTF (`.glb`) import adapter.
//!
//! glTF is a scene format: a JSON document plus one or more binary buffers,
//! optionally packed into a single GLB container. This adapter reads it
//! **bytes-only** (no file source, no HTTP): a `buffer.uri` is either a
//! `data:` base64 payload or a named refusal, never a silently empty buffer.
//!
//! Every triangle primitive becomes one mesh entity on layer `0` in model
//! space. The source frame is glTF's **metres, +Y up**; stored coordinates are
//! the CAD frame (**metres, +Z up**) via `(x, y, z) -> (x, -z, y)`, and node
//! world transforms are applied first (column-major composition).
//!
//! Losses are never silent:
//! * strip/fan primitives are skipped with [`DIAG_PRIMITIVE_MODE_UNSUPPORTED`]
//!   (points/lines with [`DIAG_PRIMITIVE_MODE_IGNORED`]), both `Partial`;
//! * a primitive without `POSITION` is [`DIAG_MESH_WITHOUT_POSITIONS`]; a file
//!   with no triangle at all is a named refusal, not an empty success;
//! * normals are dropped as a whole set when any transformed value is
//!   non-finite or zero ([`DIAG_NORMALS_DROPPED`]);
//! * glTF vertex colours are linear; per glTF 2.0 `COLOR_0` multiplies the
//!   material's linear `baseColorFactor` (that factor is applied before the
//!   linear-to-sRGB encode, and is `[1, 1, 1]` when no material is named), and
//!   RGB becomes sRGB bytes while a translucent vertex alpha cannot be stored by
//!   the neutral mesh, so it is dropped and reported
//!   ([`DIAG_VERTEX_ALPHA_DROPPED`]);
//! * the base-colour texture **is** imported when the document carries its bytes
//!   (a GLB buffer view or a base64 `data:` URI): the UV set its `texCoord` names
//!   becomes UV0 and the encoded PNG/JPEG becomes a document-owned texture
//!   (`cad_db::DrawingDatabase::textures`). A texture whose bytes live in an
//!   external file cannot be read by a bytes-only import, so the geometry keeps
//!   its UVs, no texture is referenced and the loss is reported
//!   ([`DIAG_EXTERNAL_IMAGE`], [`DIAG_TEXTURE_NOT_IMPORTED`]); an unusable UV set
//!   is dropped as a whole ([`DIAG_UVS_DROPPED`]) rather than guessed at. Other
//!   `TEXCOORD_n` sets ([`DIAG_UV_SET_IGNORED`]) and `KHR_texture_transform`
//!   ([`DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED`]) are reported, never silent;
//! * the metallic-roughness material channels **are** imported (ADR-0039 D4):
//!   `metallicFactor`/`roughnessFactor`, `emissiveFactor` (linear RGB),
//!   `occlusionTexture.strength` and the four PBR texture slots
//!   (`metallicRoughnessTexture`, `normalTexture`, `emissiveTexture`,
//!   `occlusionTexture`) become `Mesh::pbr`. `pbr` is `Some` **only** when the
//!   document's JSON declares at least one of those channels — glTF's default
//!   `metallicFactor = 1` is a format default, and applying it to a
//!   `baseColorFactor`-only asset would draw unlit CAD as black metal, so such a
//!   material stays `pbr: None`. A non-finite or out-of-range factor drops the
//!   whole channel ([`DIAG_PBR_IGNORED`]), never clamps it. Every PBR texture
//!   samples UV0: a slot on another `TEXCOORD` set is not imported
//!   ([`DIAG_UV_SET_IGNORED`]), a slot whose UV set was unusable stays unset
//!   ([`DIAG_UVS_DROPPED`]), and an external image is named
//!   ([`DIAG_EXTERNAL_IMAGE`]). A `normalTexture.scale` other than `1.0`
//!   ([`DIAG_NORMAL_SCALE_NOT_IMPORTED`]) and any
//!   `KHR_materials_emissive_strength` ([`DIAG_EMISSIVE_STRENGTH_NOT_IMPORTED`])
//!   are reported rather than guessed at;
//! * skins, animations, cameras and meshes outside the imported scene are
//!   reported, never silently ignored.
//!
//! Materials are colours only. A primitive that names **no** material keeps the
//! layer colour ([`EntityColor::ByLayer`](cad_db::EntityColor::ByLayer)): the
//! glTF default material's white factor is a format default, not an authored
//! colour, so it is not baked in. A named material's linear `baseColorFactor`
//! becomes an explicit sRGB entity colour — or, when the primitive also carries
//! `COLOR_0`, is multiplied into those vertex colours instead, so the authored
//! composition is stored once and the entity keeps `ByLayer` colour — and its
//! alpha is honoured through `alphaMode`: `OPAQUE` is imported opaque, `BLEND`
//! keeps an alpha below `1.0`, and `MASK` cutouts are imported opaque with
//! [`DIAG_ALPHA_MASK_NOT_IMPORTED`] because the neutral mesh cannot represent
//! them. The linear RGB `baseColorFactor` is *not* baked into a textured
//! primitive's geometry when that primitive has no `COLOR_0`: it stays the
//! explicit entity colour, which the renderer multiplies with the sampled
//! texture (`texture × vertex colour × entity colour`) exactly once. The
//! metallic-roughness channels a material declares are carried beside the
//! geometry in `Mesh::pbr` (ADR-0039 D1); a material that declares none keeps
//! `pbr: None` instead of receiving glTF's spec defaults.
//!
//! A document whose `asset.version` is not `2.x` is refused by name
//! ([`DIAG_UNSUPPORTED_VERSION`]); it is never read as if it were glTF 2.0.
//!
//! The writer half lives in the [`export`] module of this same connector and is
//! declared on [`DESCRIPTOR_GLTF`] as an L1 lossy export; [`GltfExporter`] and
//! [`GltfContainer`] are re-exported at the crate root for the registry.

mod buffers;
mod geometry;
mod pbr;
mod scene;
mod texture;

pub mod export;

pub use export::{GltfContainer, GltfExporter};

#[cfg(test)]
mod tests;

use cad_domain::{CadError, CadResult, Diagnostic, UnitContext};
use cad_connector_core::{
    compute_identity, ConnectorDescriptor, ExportCapability, FidelityLevel, ImportCapability,
    ImportPhase, ImportProgress, ImportProgressSink, ImportReport, ImportRequest, ImportedDrawing,
    Importer, MeshImport, NoopProgress, SpecVisibility,
};

use buffers::resolve_buffers;
use scene::SceneWalker;

/// The glTF entity `type_key` reported in capabilities and stored rows.
pub const GLTF_MESH_TYPE_KEY: &str = "GltfMesh";

/// Stable diagnostic code: a `data:` buffer payload is not valid base64.
pub const DIAG_BUFFER_BASE64: &str = "import.gltf.buffer_base64";
/// Stable diagnostic code: a decoded buffer is shorter than its `byteLength`.
pub const DIAG_BUFFER_LENGTH: &str = "import.gltf.buffer_length";
/// Stable diagnostic code: a GLB has no BIN chunk for its `Source::Bin` buffer.
pub const DIAG_MISSING_BIN: &str = "import.gltf.missing_bin";
/// Stable diagnostic code: a buffer names an external URI (bytes-only import).
pub const DIAG_EXTERNAL_BUFFER: &str = "import.gltf.external_buffer_unavailable";
/// Stable diagnostic code: the JSON/GLB container could not be parsed.
pub const DIAG_PARSE: &str = "import.gltf.parse";
/// Stable diagnostic code: the document declares no scene.
pub const DIAG_NO_SCENE: &str = "import.gltf.no_scene";
/// Stable diagnostic code: a node hierarchy closes a cycle.
pub const DIAG_NODE_CYCLE: &str = "import.gltf.node_cycle";
/// Stable diagnostic code: the hierarchy is deeper than the recursion guard.
pub const DIAG_NODE_DEPTH: &str = "import.gltf.node_depth";
/// Stable diagnostic code: a node transform is non-finite.
pub const DIAG_INVALID_TRANSFORM: &str = "import.gltf.invalid_transform";
/// Stable diagnostic code: strip/fan primitives are not triangulated.
pub const DIAG_PRIMITIVE_MODE_UNSUPPORTED: &str = "import.gltf.primitive_mode_unsupported";
/// Stable diagnostic code: point/line primitives carry no CAD geometry here.
pub const DIAG_PRIMITIVE_MODE_IGNORED: &str = "import.gltf.primitive_mode_ignored";
/// Stable diagnostic code: a triangle primitive has no `POSITION` attribute.
pub const DIAG_MESH_WITHOUT_POSITIONS: &str = "import.gltf.mesh_without_positions";
/// Stable diagnostic code: no triangle could be built from the document.
pub const DIAG_NO_GEOMETRY: &str = "import.gltf.no_geometry";
/// Stable diagnostic code: an index references a vertex outside its primitive.
pub const DIAG_INDEX_OUT_OF_RANGE: &str = "import.gltf.index_out_of_range";
/// Stable diagnostic code: a triangle index list is not a multiple of three.
pub const DIAG_TRIANGLE_INDEX_COUNT: &str = "import.gltf.triangle_index_count";
/// Stable diagnostic code: a transformed vertex is non-finite.
pub const DIAG_NON_FINITE_VERTEX: &str = "import.gltf.non_finite_vertex";
/// Stable diagnostic code: a sparse accessor cannot be expanded here.
pub const DIAG_SPARSE_ACCESSOR: &str = "import.gltf.sparse_accessor";
/// Stable diagnostic code: a normal set was unusable and dropped as a whole.
pub const DIAG_NORMALS_DROPPED: &str = "import.gltf.normals_dropped";
/// Stable diagnostic code: a vertex-colour set was unusable and dropped.
pub const DIAG_COLORS_DROPPED: &str = "import.gltf.colors_dropped";
/// Stable diagnostic code: translucent vertex alpha has no neutral-mesh channel.
pub const DIAG_VERTEX_ALPHA_DROPPED: &str = "import.gltf.vertex_alpha_dropped";
/// Stable diagnostic code: a base-colour texture's bytes are not in the document.
pub const DIAG_TEXTURE_NOT_IMPORTED: &str = "import.gltf.texture_not_imported";
/// Stable diagnostic code: a base-colour image names an external file.
pub const DIAG_EXTERNAL_IMAGE: &str = "import.gltf.external_image_unavailable";
/// Stable diagnostic code: a declared PBR factor is non-finite or out of `[0, 1]`.
pub const DIAG_PBR_IGNORED: &str = "import.gltf.pbr_ignored";
/// Stable diagnostic code: `normalTexture.scale` is not `1.0`.
pub const DIAG_NORMAL_SCALE_NOT_IMPORTED: &str = "import.gltf.normal_scale_not_imported";
/// Stable diagnostic code: `KHR_materials_emissive_strength` is not applied.
pub const DIAG_EMISSIVE_STRENGTH_NOT_IMPORTED: &str = "import.gltf.emissive_strength_not_imported";
/// Stable diagnostic code: a base-colour `KHR_texture_transform` was ignored.
pub const DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED: &str = "import.gltf.texture_transform_not_imported";
/// Stable diagnostic code: a texture declares a sampler wrap other than REPEAT,
/// which this import does not carry and the renderer will not honour.
pub const DIAG_SAMPLER_WRAP_NOT_IMPORTED: &str = "import.gltf.sampler_wrap_not_imported";
/// Stable diagnostic code: a UV set was unusable and dropped as a whole.
pub const DIAG_UVS_DROPPED: &str = "import.gltf.uvs_dropped";
/// Stable diagnostic code (note): only one UV set is read per primitive.
pub const DIAG_UV_SET_IGNORED: &str = "import.gltf.uv_set_ignored";
/// Stable diagnostic code: an image's `data:` payload is not valid base64.
pub const DIAG_IMAGE_BASE64: &str = "import.gltf.image_base64";
/// Stable diagnostic code: an image's buffer view lies outside its buffer.
pub const DIAG_IMAGE_OUT_OF_RANGE: &str = "import.gltf.image_out_of_range";
/// Stable diagnostic code: an unusable `baseColorFactor` was not applied.
pub const DIAG_MATERIAL_IGNORED: &str = "import.gltf.material_ignored";
/// Stable diagnostic code: an `alphaMode` of `MASK` (cutout) is imported opaque.
pub const DIAG_ALPHA_MASK_NOT_IMPORTED: &str = "import.gltf.alpha_mask_not_imported";
/// Stable refusal code: the document's `asset.version` is not glTF 2.x.
pub const DIAG_UNSUPPORTED_VERSION: &str = "import.gltf.unsupported_version";
/// Stable diagnostic code: a degenerate triangle was dropped.
pub const DIAG_DEGENERATE_TRIANGLE: &str = "import.gltf.degenerate_triangle";
/// Stable diagnostic code (note): glTF coordinates are metres.
pub const DIAG_UNITS_METRES: &str = "import.gltf.units_metres";
/// Stable diagnostic code (note): the glTF +Y-up frame was converted to +Z up.
pub const DIAG_FRAME_CONVERTED: &str = "import.gltf.frame_converted";
/// Stable diagnostic code: skins are not imported.
pub const DIAG_SKIN_IGNORED: &str = "import.gltf.skin_ignored";
/// Stable diagnostic code: animations are not imported.
pub const DIAG_ANIMATION_IGNORED: &str = "import.gltf.animation_ignored";
/// Stable diagnostic code: cameras are not imported.
pub const DIAG_CAMERA_IGNORED: &str = "import.gltf.camera_ignored";
/// Stable diagnostic code: a mesh is referenced by no node in the scene.
pub const DIAG_UNREFERENCED_MESH: &str = "import.gltf.unreferenced_mesh";

/// The glTF connector descriptor.
///
/// Owned by this crate. The writer half ([`export`]) emits glTF 2.0 at
/// L1; the format cannot carry `cad-db`'s analytic curves, layers or units, so
/// the export direction is lossy and every conversion is reported.
///
/// `spec` is [`SpecVisibility::Public`]: glTF 2.0 is the published Khronos
/// specification (`THIRD_PARTY_NOTICES.md`:65-66).
pub const DESCRIPTOR_GLTF: ConnectorDescriptor = ConnectorDescriptor {
    id: "gltf",
    name_key: "connector.gltf.name",
    extensions: &["gltf", "glb"],
    spec: SpecVisibility::Public,
    import: Some(ImportCapability {
        versions: "glTF 2.0 (.gltf JSON, .glb binary)",
        container: "gltf",
    }),
    export: Some(ExportCapability {
        versions: "glTF 2.0 (GLB + JSON .gltf)",
        level: FidelityLevel::L1,
        lossy: true,
        experimental: false,
    }),
    declared_classes: &[],
};

/// Conservatively identify glTF bytes.
///
/// This delegates to the shared detector so the connector's answer can never
/// drift from the dispatcher's: a GLB needs the `glTF` magic plus a version-2
/// header, and a JSON `.gltf` must be a JSON object carrying the `asset` and
/// `version` keys. Anything else is `false`, never a guess.
pub fn sniff(bytes: &[u8]) -> bool {
    matches!(
        cad_connector_core::detect_format(bytes),
        cad_connector_core::DrawingFormat::Gltf
    )
}

/// The document JSON of a `.gltf`, or the JSON chunk of a GLB.
///
/// A GLB wraps the document in a chunked container, so the JSON is extracted from
/// it; for a JSON `.gltf` the input bytes are the document.
fn document_json(bytes: &[u8]) -> CadResult<std::borrow::Cow<'_, [u8]>> {
    if bytes.starts_with(b"glTF") {
        let glb = gltf::binary::Glb::from_slice(bytes)
            .map_err(|error| CadError::CorruptData(format!("{DIAG_PARSE}: {error}")))?;
        Ok(glb.json)
    } else {
        Ok(std::borrow::Cow::Borrowed(bytes))
    }
}

/// Refuse a document whose `asset.version` is not glTF 2.x before any parse.
///
/// A missing or non-string `asset.version` is the same named refusal as a `1.x`
/// one.
fn check_asset_version(document: &serde_json::Value) -> CadResult<()> {
    let version = document
        .get("asset")
        .and_then(|asset| asset.get("version"))
        .and_then(serde_json::Value::as_str);
    match version {
        Some(version) if version.starts_with("2.") => Ok(()),
        Some(version) => Err(CadError::Unsupported(format!(
            "{DIAG_UNSUPPORTED_VERSION}: asset.version is {version:?}; only glTF 2.x documents \
             are imported"
        ))),
        None => Err(CadError::Unsupported(format!(
            "{DIAG_UNSUPPORTED_VERSION}: the document declares no string asset.version; only \
             glTF 2.x documents are imported"
        ))),
    }
}

/// The glTF importer.
#[derive(Default)]
pub struct GltfImporter;

impl GltfImporter {
    pub fn new() -> Self {
        Self
    }
}

impl Importer for GltfImporter {
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

        let started = web_time::Instant::now();
        let identity = compute_identity(&request.bytes);
        progress.report(ImportProgress {
            phase: ImportPhase::Reading,
            entities_done: 0,
            entities_total: None,
            bytes: Some(request.bytes.len() as u64),
            message: None,
        });
        // One JSON parse serves the version gate, the extension scan and the
        // authored-PBR scan; the `gltf` crate is built without the typed
        // `KHR_texture_transform`/`KHR_materials_emissive_strength` accessors and
        // its getters answer with spec defaults, so the authored keys are read
        // from this value.
        let json = document_json(&request.bytes)?;
        let json: serde_json::Value = serde_json::from_slice(&json)
            .map_err(|error| CadError::CorruptData(format!("{DIAG_PARSE}: {error}")))?;
        check_asset_version(&json)?;
        let transformed_materials = texture::transform_materials(&json);
        let pbr_declarations = texture::pbr_declarations(&json);
        // The renderer samples mesh material textures with REPEAT (glTF's
        // default). A document that declares another wrap cannot be stored in
        // `Mesh`/`MeshPbr` or `.ymodel`, so it is named per imported slot rather
        // than silently reinterpreted.
        let wrap_declarations = texture::wrap_declarations(&json);

        let gltf = gltf::Gltf::from_slice(&request.bytes)
            .map_err(|error| CadError::CorruptData(format!("{DIAG_PARSE}: {error}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        let buffers = resolve_buffers(&gltf)?;
        let Some(scene) = gltf
            .document
            .default_scene()
            .or_else(|| gltf.document.scenes().next())
        else {
            return Err(CadError::Unsupported(format!(
                "{DIAG_NO_SCENE}: the document declares no scene"
            )));
        };

        progress.report(ImportProgress::at(ImportPhase::Parsing, 0, None));

        let mut import = MeshImport::new(request, GLTF_MESH_TYPE_KEY)?;
        let mut walker = SceneWalker::new(
            &gltf.document,
            &buffers,
            &request.limits,
            cancelled,
            &transformed_materials,
            &pbr_declarations,
            &wrap_declarations,
        );
        let mut stack = Vec::new();
        for node in scene.nodes() {
            walker.walk(&node, &scene::IDENTITY, &mut stack, &mut import)?;
        }
        walker.record_ignored(&mut import);

        if import.entity_count() == 0 {
            if walker.saw_mesh_without_positions {
                return Err(CadError::CorruptData(format!(
                    "{DIAG_MESH_WITHOUT_POSITIONS}: no triangle primitive carries a POSITION \
                     attribute"
                )));
            }
            return Err(CadError::Unsupported(format!(
                "{DIAG_NO_GEOMETRY}: the document produced no triangle geometry"
            )));
        }

        import.record(
            Diagnostic {
                object: None,
                code: DIAG_UNITS_METRES.into(),
                message: "glTF coordinates are metres; stored values keep the metre unit".into(),
            },
            None,
        );
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_FRAME_CONVERTED.into(),
                message: "glTF +Y up was converted to the CAD +Z up frame as (x, y, z) -> \
                          (x, -z, y)"
                    .into(),
            },
            None,
        );

        let entity_total = import.entity_count();
        progress.report(ImportProgress::at(
            ImportPhase::Entities,
            entity_total,
            Some(entity_total),
        ));
        progress.report(ImportProgress::at(
            ImportPhase::Finishing,
            entity_total,
            Some(entity_total),
        ));

        let outcome = import.finish()?;
        let report = ImportReport {
            identity,
            capabilities: outcome.capabilities,
            completeness: outcome.completeness,
            diagnostics: outcome.diagnostics,
            connector: None,
            parse_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            object_xdata: Vec::new(),
        };

        Ok(ImportedDrawing {
            database: outcome.database,
            units: UnitContext::from_meter(),
            report,
            vendor_records: Vec::new(),
        })
    }
}
