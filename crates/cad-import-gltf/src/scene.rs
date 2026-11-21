//! Scene traversal: node world transforms into neutral CAD meshes.
//!
//! glTF node matrices are **column-major** (`gltf::scene::Transform::matrix`),
//! and a child's world transform is `parent_world * local`. Positions are
//! transformed in the glTF frame first (metres, +Y up) and then mapped to the
//! CAD frame (metres, +Z up) as `(x, y, z) -> (x, -z, y)`.

use std::collections::{BTreeMap, BTreeSet};

use cad_db::{EntityColor, EntityRenderAttributes, EntityTransparency};
use cad_domain::{CadError, CadResult, Diagnostic, Mesh, ObjectId, Point3};
use cad_import_core::{ImportLimits, MeshImport};
use gltf::{
    material::AlphaMode, mesh::Mode, texture::Info as TextureInfo, Node, Primitive, Semantic,
};

use crate::buffers::ResolvedBuffer;
use crate::geometry::{
    apply, is_zero_area, local_matrix, multiply, normal_matrix, to_cad_frame, transform_normals,
};
use crate::pbr::{PbrChannels, PbrLosses};
use crate::texture::{
    refusal_marker, resolve_texture_info, PbrDecl, TextureImage, WrapDeclaration, WrapLosses,
};
use crate::{
    DIAG_ALPHA_MASK_NOT_IMPORTED, DIAG_ANIMATION_IGNORED, DIAG_CAMERA_IGNORED, DIAG_COLORS_DROPPED,
    DIAG_DEGENERATE_TRIANGLE, DIAG_EXTERNAL_IMAGE, DIAG_INDEX_OUT_OF_RANGE, DIAG_INVALID_TRANSFORM,
    DIAG_MATERIAL_IGNORED, DIAG_MESH_WITHOUT_POSITIONS, DIAG_NODE_CYCLE, DIAG_NODE_DEPTH,
    DIAG_NON_FINITE_VERTEX, DIAG_NORMALS_DROPPED, DIAG_PRIMITIVE_MODE_IGNORED,
    DIAG_PRIMITIVE_MODE_UNSUPPORTED, DIAG_SKIN_IGNORED, DIAG_SPARSE_ACCESSOR,
    DIAG_TEXTURE_NOT_IMPORTED, DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED, DIAG_TRIANGLE_INDEX_COUNT,
    DIAG_UNREFERENCED_MESH, DIAG_UVS_DROPPED, DIAG_UV_SET_IGNORED, DIAG_VERTEX_ALPHA_DROPPED,
};

/// The maximum node-hierarchy depth accepted before the recursion guard trips.
const MAX_NODE_DEPTH: usize = 64;

/// The identity affine applied to scene roots (their local matrix runs once).
pub(crate) const IDENTITY: [[f64; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Walks one scene and inserts one entity per triangle primitive.
pub(crate) struct SceneWalker<'a> {
    document: &'a gltf::Document,
    buffers: &'a [ResolvedBuffer<'a>],
    limits: &'a ImportLimits,
    cancelled: &'a dyn Fn() -> bool,
    /// Material indices whose base-colour texture info carries
    /// `KHR_texture_transform` (the transform is not applied).
    transformed_materials: &'a BTreeSet<usize>,
    /// Every material's authored PBR declaration, read from the document JSON
    /// (ADR-0039 D4). A missing entry means "no authored PBR" → `pbr: None`.
    pbr_declarations: &'a BTreeMap<usize, PbrDecl>,
    /// The non-REPEAT sampler wrap each texture declares, by texture index.
    /// The renderer samples REPEAT, so any entry here is a named loss.
    wrap_declarations: &'a BTreeMap<usize, WrapDeclaration>,
    referenced_meshes: BTreeSet<usize>,
    /// `TEXCOORD_n` sets that were present but not the imported set.
    ignored_uv_sets: BTreeSet<u32>,
    /// `true` when at least one triangle primitive had no usable positions.
    pub(crate) saw_mesh_without_positions: bool,
}

impl<'a> SceneWalker<'a> {
    pub(crate) fn new(
        document: &'a gltf::Document,
        buffers: &'a [ResolvedBuffer<'a>],
        limits: &'a ImportLimits,
        cancelled: &'a dyn Fn() -> bool,
        transformed_materials: &'a BTreeSet<usize>,
        pbr_declarations: &'a BTreeMap<usize, PbrDecl>,
        wrap_declarations: &'a BTreeMap<usize, WrapDeclaration>,
    ) -> Self {
        SceneWalker {
            document,
            buffers,
            limits,
            cancelled,
            transformed_materials,
            pbr_declarations,
            wrap_declarations,
            referenced_meshes: BTreeSet::new(),
            ignored_uv_sets: BTreeSet::new(),
            saw_mesh_without_positions: false,
        }
    }

    pub(crate) fn walk(
        &mut self,
        node: &Node<'_>,
        parent_world: &[[f64; 4]; 4],
        stack: &mut Vec<usize>,
        import: &mut MeshImport,
    ) -> CadResult<()> {
        if (self.cancelled)() {
            return Err(CadError::Cancelled);
        }
        let index = node.index();
        if stack.contains(&index) {
            return Err(CadError::CorruptData(format!(
                "{DIAG_NODE_CYCLE}: glTF node {index} closes a hierarchy cycle"
            )));
        }
        if stack.len() >= MAX_NODE_DEPTH {
            return Err(CadError::CorruptData(format!(
                "{DIAG_NODE_DEPTH}: the node hierarchy is deeper than {MAX_NODE_DEPTH} nodes"
            )));
        }
        stack.push(index);

        let world = multiply(parent_world, &local_matrix(node)?);
        if !world.iter().flatten().all(|value| value.is_finite()) {
            return Err(CadError::CorruptData(format!(
                "{DIAG_INVALID_TRANSFORM}: the world transform of glTF node {index} is not finite"
            )));
        }

        if let Some(mesh) = node.mesh() {
            self.referenced_meshes.insert(mesh.index());
            for primitive in mesh.primitives() {
                if (self.cancelled)() {
                    return Err(CadError::Cancelled);
                }
                self.import_primitive(&primitive, mesh.index(), &world, import)?;
            }
        }

        for child in node.children() {
            self.walk(&child, &world, stack, import)?;
        }
        stack.pop();
        Ok(())
    }

    fn import_primitive(
        &mut self,
        primitive: &Primitive<'_>,
        mesh_index: usize,
        world: &[[f64; 4]; 4],
        import: &mut MeshImport,
    ) -> CadResult<()> {
        let label = format!("mesh {mesh_index} primitive {}", primitive.index());
        match primitive.mode() {
            Mode::Triangles => {}
            Mode::TriangleStrip | Mode::TriangleFan => {
                let mode = primitive.mode();
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_PRIMITIVE_MODE_UNSUPPORTED.into(),
                        message: format!(
                            "{label}: primitive mode {mode:?} is not triangulated; the primitive \
                             was skipped"
                        ),
                    },
                    Some(format!("{label}: {mode:?} primitives were skipped")),
                );
                return Ok(());
            }
            other => {
                import.record(
                    Diagnostic {
                        object: None,
                        code: DIAG_PRIMITIVE_MODE_IGNORED.into(),
                        message: format!(
                            "{label}: primitive mode {other:?} carries no triangle geometry and \
                             was ignored"
                        ),
                    },
                    Some(format!("{label}: {other:?} primitives were ignored")),
                );
                return Ok(());
            }
        }

        let mut build = build_primitive(
            primitive,
            world,
            self.buffers,
            self.limits,
            self.transformed_materials,
            self.pbr_declarations,
            self.wrap_declarations,
            import,
        )?;
        for set in &build.ignored_uv_sets {
            self.ignored_uv_sets.insert(*set);
        }
        if build.missing_positions {
            self.saw_mesh_without_positions = true;
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_MESH_WITHOUT_POSITIONS.into(),
                    message: format!(
                        "{label}: a triangle primitive has no usable POSITION attribute; the \
                         primitive was skipped"
                    ),
                },
                Some(format!(
                    "{label}: a triangle primitive had no POSITION attribute"
                )),
            );
            return Ok(());
        }

        if let Some(mesh) = build.mesh.take() {
            if import.entity_count() >= self.limits.max_entities {
                return Err(CadError::InvalidInput(format!(
                    "glTF has more than {} drawable mesh primitive(s), over the {} entity import \
                     limit",
                    import.entity_count(),
                    self.limits.max_entities
                )));
            }
            let object = import.reserve_object()?;
            record_losses(import, Some(object), &label, &build);
            import.push_mesh_with_attributes(
                object,
                mesh,
                EntityRenderAttributes {
                    clip: None,
                    color: build.color,
                    transparency: build.transparency,
                    ..Default::default()
                },
            )?;
        } else {
            record_losses(import, None, &label, &build);
        }
        Ok(())
    }

    /// Report document-level content this importer could not represent.
    pub(crate) fn record_ignored(&self, import: &mut MeshImport) {
        if self.document.skins().next().is_some() {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_SKIN_IGNORED.into(),
                    message: "skins/joints are not imported; geometry is stored in its rest pose"
                        .into(),
                },
                Some("skins are not imported; geometry is stored in its rest pose".into()),
            );
        }
        if self.document.animations().next().is_some() {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_ANIMATION_IGNORED.into(),
                    message: "animations are not imported".into(),
                },
                Some("animations are not imported".into()),
            );
        }
        if self.document.cameras().next().is_some() {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_CAMERA_IGNORED.into(),
                    message: "glTF cameras are not imported".into(),
                },
                Some("glTF cameras are not imported".into()),
            );
        }
        let unreferenced: Vec<usize> = self
            .document
            .meshes()
            .map(|mesh| mesh.index())
            .filter(|index| !self.referenced_meshes.contains(index))
            .collect();
        if !unreferenced.is_empty() {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_UNREFERENCED_MESH.into(),
                    message: format!(
                        "mesh(es) {unreferenced:?} are referenced by no node in the imported \
                         scene and were skipped"
                    ),
                },
                Some("at least one mesh is referenced by no node in the scene".into()),
            );
        }
        if !self.ignored_uv_sets.is_empty() {
            let sets: Vec<u32> = self.ignored_uv_sets.iter().copied().collect();
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_UV_SET_IGNORED.into(),
                    message: format!(
                        "only one UV set per primitive is imported (the set its textures sample); \
                         TEXCOORD set(s) {sets:?} were not read"
                    ),
                },
                // A note, not a loss: UV1+ has no representation in the domain and
                // nothing in the document asked for it to be drawn (ADR-0038 D1).
                None,
            );
        }
    }
}

/// What one triangle primitive produced, plus every loss it incurred.
#[derive(Default)]
struct PrimitiveBuild {
    mesh: Option<Mesh>,
    color: EntityColor,
    transparency: EntityTransparency,
    missing_positions: bool,
    degenerate: usize,
    normals_dropped: usize,
    colors_dropped: usize,
    /// Vertices whose stored alpha was below `1.0` (the mesh has RGB only).
    alpha_dropped: usize,
    /// UV sets the primitive carried but that were not the imported set.
    ignored_uv_sets: Vec<u32>,
    /// The base-colour UV set was unusable and dropped as a whole.
    uvs_dropped: usize,
    /// The base-colour image names bytes this bytes-only import cannot read.
    texture_unavailable: Option<String>,
    /// `register_texture` refused the bytes: the stable marker and its message.
    texture_refused: Option<(String, String)>,
    /// The imported texture's info carried `KHR_texture_transform`.
    texture_transform_not_imported: bool,
    /// The material's base-colour texture bytes are outside the document.
    texture_not_imported: bool,
    material_invalid: bool,
    alpha_mask_not_imported: bool,
    /// Every PBR loss this primitive incurred (ADR-0039 D4).
    pbr: PbrLosses,
    /// Non-REPEAT sampler wraps the imported texture slots declare (the
    /// renderer samples REPEAT; base colour and PBR slots share this set).
    sampler_wraps: WrapLosses,
}

fn record_losses(
    import: &mut MeshImport,
    object: Option<ObjectId>,
    label: &str,
    build: &PrimitiveBuild,
) {
    if build.degenerate > 0 {
        import.record(
            Diagnostic {
                object,
                code: DIAG_DEGENERATE_TRIANGLE.into(),
                message: format!(
                    "{label}: {} degenerate triangle(s) (repeated vertex or zero area) were \
                     dropped",
                    build.degenerate
                ),
            },
            Some(format!(
                "{label}: {} degenerate triangle(s) were dropped",
                build.degenerate
            )),
        );
    }
    if build.normals_dropped > 0 {
        import.record(
            Diagnostic {
                object,
                code: DIAG_NORMALS_DROPPED.into(),
                message: format!(
                    "{label}: the vertex-normal set was unusable after its world transform and \
                     was dropped as a whole"
                ),
            },
            Some(format!("{label}: the vertex-normal set was dropped")),
        );
    }
    if build.colors_dropped > 0 {
        import.record(
            Diagnostic {
                object,
                code: DIAG_COLORS_DROPPED.into(),
                message: format!(
                    "{label}: the vertex-colour set did not match the vertex count or carried \
                     non-finite values and was dropped"
                ),
            },
            Some(format!("{label}: the vertex-colour set was dropped")),
        );
    }
    if build.alpha_dropped > 0 {
        import.record(
            Diagnostic {
                object,
                code: DIAG_VERTEX_ALPHA_DROPPED.into(),
                message: format!(
                    "{label}: {} vertex colour(s) carry alpha below 1.0; the neutral mesh stores \
                     RGB only, so that alpha was dropped",
                    build.alpha_dropped
                ),
            },
            Some(format!(
                "{label}: translucent vertex alpha was dropped ({} vertices)",
                build.alpha_dropped
            )),
        );
    }
    if build.uvs_dropped > 0 {
        import.record(
            Diagnostic {
                object,
                code: DIAG_UVS_DROPPED.into(),
                message: format!(
                    "{label}: the UV set the material's base-color texture names did not have \
                     one finite pair per vertex and was dropped as a whole; the primitive keeps \
                     its geometry and no texture is referenced"
                ),
            },
            Some(format!("{label}: the base-colour UV set was dropped")),
        );
    }
    if build.texture_not_imported {
        import.record(
            Diagnostic {
                object,
                code: DIAG_TEXTURE_NOT_IMPORTED.into(),
                message: format!(
                    "{label}: the material has a base-color texture whose bytes are not inside \
                     the document; the primitive keeps its UVs, the base-color factor is used \
                     instead"
                ),
            },
            Some(format!(
                "{label}: a base-colour texture's bytes were unavailable"
            )),
        );
    }
    if let Some(uri) = &build.texture_unavailable {
        import.record(
            Diagnostic {
                object,
                code: DIAG_EXTERNAL_IMAGE.into(),
                message: format!(
                    "{label}: the base-color image is `{uri}`, which this bytes-only import \
                     cannot read; the UVs are kept and no texture is referenced"
                ),
            },
            Some(format!("{label}: a base-colour image is an external file")),
        );
    }
    if let Some((code, message)) = &build.texture_refused {
        import.record(
            Diagnostic {
                object,
                code: code.clone(),
                message: format!("{label}: {message}"),
            },
            Some(format!("{label}: a base-colour texture was refused")),
        );
    }
    if build.texture_transform_not_imported {
        import.record(
            Diagnostic {
                object,
                code: DIAG_TEXTURE_TRANSFORM_NOT_IMPORTED.into(),
                message: format!(
                    "{label}: the base-color texture info carries KHR_texture_transform; the \
                     transform is not applied and the UVs are stored as authored"
                ),
            },
            Some(format!(
                "{label}: a base-colour KHR_texture_transform was not applied"
            )),
        );
    }
    if build.material_invalid {
        import.record(
            Diagnostic {
                object,
                code: DIAG_MATERIAL_IGNORED.into(),
                message: format!(
                    "{label}: the material baseColorFactor is non-finite or outside [0, 1]; the \
                     entity keeps ByLayer colour and opacity"
                ),
            },
            Some(format!("{label}: an unusable material colour was ignored")),
        );
    }
    if build.alpha_mask_not_imported {
        import.record(
            Diagnostic {
                object,
                code: DIAG_ALPHA_MASK_NOT_IMPORTED.into(),
                message: format!(
                    "{label}: the material uses alphaMode MASK; cutout masks are not \
                     representable, so the primitive was imported opaque"
                ),
            },
            Some(format!(
                "{label}: an alphaMode MASK cutout was imported opaque"
            )),
        );
    }
    build.pbr.record(import, object, label);
    build
        .sampler_wraps
        .record_diagnostics(import, object, label);
}

#[allow(clippy::too_many_arguments)]
fn build_primitive(
    primitive: &Primitive<'_>,
    world: &[[f64; 4]; 4],
    buffers: &[ResolvedBuffer<'_>],
    limits: &ImportLimits,
    transformed_materials: &BTreeSet<usize>,
    pbr_declarations: &BTreeMap<usize, PbrDecl>,
    wrap_declarations: &BTreeMap<usize, WrapDeclaration>,
    import: &mut MeshImport,
) -> CadResult<PrimitiveBuild> {
    let style = material_style(primitive);
    // glTF 2.0: `COLOR_0` multiplies the material's linear `baseColorFactor`.
    // A primitive with no material (or an unusable factor) uses `[1, 1, 1]`, so
    // the vertex colours are stored as authored.
    let color_factor = style.factor.unwrap_or([1.0, 1.0, 1.0]);
    let mut build = PrimitiveBuild {
        color: style.color,
        transparency: style.transparency,
        material_invalid: style.material_invalid,
        alpha_mask_not_imported: style.alpha_mask_not_imported,
        ..Default::default()
    };

    // PBR channels (ADR-0039 D4). Only a material whose JSON declares a PBR
    // channel becomes a `MeshPbr`: the crate's getters would otherwise answer
    // with glTF's spec defaults (`metallicFactor = 1`), which would draw an
    // unlit `baseColorFactor`-only asset as black metal. The scalars are
    // validated before any texture byte is registered, so an unusable factor
    // leaves no orphan image in the document.
    let declaration = style
        .material_index
        .and_then(|index| pbr_declarations.get(&index));
    let pbr_channels = declaration
        .filter(|decl| decl.declared)
        .map(|_| PbrChannels::read(primitive));
    if let Some(decl) = declaration {
        build.pbr = PbrLosses::declared(decl);
    }
    if let Some(channels) = &pbr_channels {
        if let Some(invalid) = channels.invalid_scalars() {
            build.pbr.ignore(invalid);
        }
        build
            .pbr
            .set_normal_scale(channels.unauthored_normal_scale());
    }

    // The one UV set this primitive reads. The base-colour slot keeps owning it
    // when it names one (ADR-0038 D6); otherwise a PBR texture slot that samples
    // set 0 makes set 0 the imported set, because every PBR channel is stored
    // against UV0.
    let uv_set = style
        .base_color_texture
        .as_ref()
        .map(TextureInfo::tex_coord)
        .or_else(|| pbr_channels.as_ref().and_then(PbrChannels::uv_set));

    // `accessor::Iter` returns `None` for sparse accessors (they have no buffer
    // view), which would make positions "absent" and indices look like triangle
    // soup. Refuse by name instead of misreading them. The UV set this primitive
    // samples is part of the list for the same reason: a sparse `TEXCOORD_n`
    // would otherwise read as "no UV set" and be indistinguishable from an
    // absent one.
    let mut accessors = vec![
        primitive.get(&Semantic::Positions),
        primitive.get(&Semantic::Normals),
        primitive.get(&Semantic::Colors(0)),
        primitive.indices(),
    ];
    if let Some(set) = uv_set {
        accessors.push(primitive.get(&Semantic::TexCoords(set)));
    }
    for accessor in accessors.into_iter().flatten() {
        if accessor.sparse().is_some() {
            return Err(CadError::Unsupported(format!(
                "{DIAG_SPARSE_ACCESSOR}: a sparse glTF accessor is not supported"
            )));
        }
    }

    if primitive.get(&Semantic::Positions).is_none() {
        build.missing_positions = true;
        return Ok(build);
    }
    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|b| b.as_slice()));
    let Some(positions) = reader.read_positions() else {
        build.missing_positions = true;
        return Ok(build);
    };
    let local: Vec<Point3> = positions
        .map(|position| Point3 {
            x: f64::from(position[0]),
            y: f64::from(position[1]),
            z: f64::from(position[2]),
        })
        .collect();
    if local.len() > u32::MAX as usize {
        return Err(CadError::InvalidInput(format!(
            "a glTF primitive has {} vertices, over the u32 mesh index range",
            local.len()
        )));
    }

    let mut vertices = Vec::with_capacity(local.len());
    for point in &local {
        let world_point = apply(world, *point);
        if !world_point.x.is_finite() || !world_point.y.is_finite() || !world_point.z.is_finite() {
            return Err(CadError::CorruptData(format!(
                "{DIAG_NON_FINITE_VERTEX}: a glTF vertex is non-finite after its node transform"
            )));
        }
        vertices.push(to_cad_frame(world_point));
    }

    let indices = match primitive.indices() {
        Some(_) => match reader.read_indices() {
            Some(indices) => indices.into_u32().collect::<Vec<u32>>(),
            None => {
                return Err(CadError::CorruptData(format!(
                    "{DIAG_TRIANGLE_INDEX_COUNT}: an index accessor is not an unsigned scalar"
                )));
            }
        },
        None => {
            if !vertices.len().is_multiple_of(3) {
                return Err(CadError::CorruptData(format!(
                    "{DIAG_TRIANGLE_INDEX_COUNT}: a non-indexed triangle primitive has {} \
                     vertices, not a multiple of 3",
                    vertices.len()
                )));
            }
            (0..vertices.len() as u32).collect()
        }
    };
    if !indices.len().is_multiple_of(3) {
        return Err(CadError::CorruptData(format!(
            "{DIAG_TRIANGLE_INDEX_COUNT}: a triangle primitive has {} indices, not a multiple of 3",
            indices.len()
        )));
    }
    check_mesh_limits(limits, indices.len() / 3)?;

    let vertex_count = vertices.len() as u32;
    let mut triangles = Vec::with_capacity(indices.len() / 3);
    for corner in indices.as_chunks::<3>().0 {
        if corner.iter().any(|index| *index >= vertex_count) {
            return Err(CadError::CorruptData(format!(
                "{DIAG_INDEX_OUT_OF_RANGE}: a triangle references a vertex outside its \
                 {vertex_count}-vertex primitive"
            )));
        }
        if corner[0] == corner[1]
            || corner[1] == corner[2]
            || corner[0] == corner[2]
            || is_zero_area(*corner, &vertices)
        {
            build.degenerate += 1;
            continue;
        }
        triangles.push(*corner);
    }

    let mut normals = Vec::new();
    if primitive.get(&Semantic::Normals).is_some() {
        match reader.read_normals() {
            Some(read) => {
                let raw: Vec<Point3> = read
                    .map(|normal| Point3 {
                        x: f64::from(normal[0]),
                        y: f64::from(normal[1]),
                        z: f64::from(normal[2]),
                    })
                    .collect();
                let usable = if raw.len() == vertices.len() {
                    normal_matrix(world).and_then(|matrix| transform_normals(&raw, &matrix))
                } else {
                    None
                };
                match usable {
                    Some(transformed) => normals = transformed,
                    None => build.normals_dropped += 1,
                }
            }
            None => build.normals_dropped += 1,
        }
    }

    let mut colors = Vec::new();
    if primitive.get(&Semantic::Colors(0)).is_some() {
        match reader.read_colors(0) {
            Some(read) => {
                let rgba: Vec<[f32; 4]> = read.into_rgba_f32().collect();
                if rgba.len() != vertices.len()
                    || rgba.iter().flatten().any(|value| !value.is_finite())
                {
                    build.colors_dropped += 1;
                } else {
                    let mut translucent = 0usize;
                    colors = rgba
                        .into_iter()
                        .map(|color| {
                            if color[3] < 1.0 {
                                translucent += 1;
                            }
                            [
                                linear_to_srgb_byte(color[0] * color_factor[0]),
                                linear_to_srgb_byte(color[1] * color_factor[1]),
                                linear_to_srgb_byte(color[2] * color_factor[2]),
                            ]
                        })
                        .collect();
                    build.alpha_dropped = translucent;
                }
            }
            None => build.colors_dropped += 1,
        }
    }
    if !colors.is_empty() {
        // Per-vertex colours drive the display. The material's linear RGB factor
        // is already multiplied into them (glTF 2.0 `COLOR_0` semantics), so an
        // explicit entity colour would double-apply it. The material alpha is
        // still stored.
        build.color = EntityColor::ByLayer;
    }
    if triangles.is_empty() {
        return Ok(build);
    }

    // Textures (ADR-0038 D6, ADR-0039 D4). Only the one UV set this primitive
    // reads is stored, and only the encoded bytes the document itself carries can
    // become a document-owned texture. UVs and a texture key are stored together
    // or not at all: a texture with no UV could never be sampled, so `cad-db`
    // refuses that combination and this reader never builds it.
    let mut uvs = Vec::new();
    let mut texture = None;
    if let Some(set) = uv_set {
        for (semantic, _) in primitive.attributes() {
            if let Semantic::TexCoords(other) = semantic {
                if other != set {
                    build.ignored_uv_sets.push(other);
                }
            }
        }
        let values: Option<Vec<[f64; 2]>> = reader.read_tex_coords(set).map(|read| {
            read.into_f32()
                .map(|uv| [f64::from(uv[0]), f64::from(uv[1])])
                .collect()
        });
        // The whole set is kept or dropped: a partial list or a `NaN` component
        // would put the texture somewhere the document never authored it.
        match values {
            Some(values)
                if values.len() == vertices.len()
                    && values.iter().flatten().all(|value| value.is_finite()) =>
            {
                uvs = values;
            }
            _ => build.uvs_dropped += 1,
        }
    }
    if !uvs.is_empty() {
        if let Some(info) = &style.base_color_texture {
            match resolve_texture_info(info, buffers)? {
                TextureImage::Embedded(bytes) => match import.register_texture(&bytes) {
                    Ok(key) => {
                        texture = Some(key);
                        build.texture_transform_not_imported = style
                            .material_index
                            .is_some_and(|index| transformed_materials.contains(&index));
                        // The bytes are resident, so the renderer will sample the
                        // texture with its REPEAT sampler; name any other wrap.
                        build.sampler_wraps.record(
                            "baseColorTexture",
                            &info.texture(),
                            wrap_declarations,
                        );
                    }
                    Err(error) => {
                        // A budget/format refusal loses the texture only: the
                        // geometry and its UVs stay, reported by the refusal's own
                        // stable marker.
                        let message = error.to_string();
                        build.texture_refused =
                            Some((refusal_marker(&message).to_string(), message));
                    }
                },
                TextureImage::Unavailable { uri } => {
                    build.texture_unavailable = Some(uri);
                    build.texture_not_imported = true;
                }
            }
        }
    }

    // PBR channels (ADR-0039 D3/D4). Only a material with usable scalars gets a
    // `MeshPbr`; each texture slot then either imports its bytes or is reported.
    // The channels share UV0, so a slot on any other `TEXCOORD` set cannot be
    // sampled and is left unset (`uv_set_ignored`), and every slot stays unset
    // when the UV set was unusable (`uvs_dropped`) — the scalars still apply.
    let mut pbr = None;
    if let Some(channels) = pbr_channels.as_ref().filter(|_| !build.pbr.is_ignored()) {
        pbr = channels.import(
            uv_set,
            !uvs.is_empty(),
            buffers,
            import,
            &mut build.pbr,
            wrap_declarations,
            &mut build.sampler_wraps,
        )?;
        let ignored = std::mem::take(&mut build.pbr.ignored_uv_sets);
        build.ignored_uv_sets.extend(ignored);
    }

    build.mesh = Some(Mesh {
        vertices,
        triangles,
        normals,
        face_sources: Vec::new(),
        colors,
        uvs,
        texture,
        pbr,
    });
    Ok(build)
}

/// What the primitive's material contributes, plus every loss it incurs.
#[derive(Default)]
struct MaterialStyle<'a> {
    color: EntityColor,
    transparency: EntityTransparency,
    /// The named material's linear RGB factor, when it is usable; multiplied
    /// into any `COLOR_0` vertex colours. `None` means `[1, 1, 1]`.
    factor: Option<[f32; 3]>,
    /// The material's base-colour texture reference, when it names one.
    base_color_texture: Option<TextureInfo<'a>>,
    /// The named material's JSON index (the default material has none).
    material_index: Option<usize>,
    material_invalid: bool,
    alpha_mask_not_imported: bool,
}

/// The primitive's material as stored units, plus the losses that material adds.
///
/// A primitive that names **no** material keeps `ByLayer` colour and opacity:
/// `primitive.material()` would otherwise answer with the glTF *default*
/// material, whose white factor is a format default rather than an authored
/// colour. Only a material with a JSON index is read.
///
/// glTF colours are **linear**. The RGB factor of a named material becomes sRGB
/// bytes (the exact IEC transfer function). Its alpha is honoured through
/// `alphaMode`:
///
/// * `OPAQUE` is imported opaque and the factor's alpha is ignored;
/// * `BLEND` stores an explicit opacity only below `1.0` (`1.0` stays opaque);
/// * `MASK` cutouts cannot be represented, so the primitive is imported opaque
///   and the loss is reported.
///
/// The linear RGB factor is also kept in [`MaterialStyle::factor`] so a
/// primitive with `COLOR_0` multiplies it into its vertex colours exactly once
/// (the glTF 2.0 rule) instead of discarding it. It is equally not baked into a
/// **textured** primitive with no `COLOR_0`: the renderer composes
/// `texture x vertex colour x entity colour`, so the factor stays the entity
/// colour there and is applied exactly once.
///
/// A factor outside the spec range is not clamped into a plausible colour: the
/// entity keeps `ByLayer` and the loss is reported.
fn material_style<'a>(primitive: &Primitive<'a>) -> MaterialStyle<'a> {
    let material = primitive.material();
    let Some(index) = material.index() else {
        return MaterialStyle::default();
    };
    let pbr = material.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    let mut style = MaterialStyle {
        base_color_texture: pbr.base_color_texture(),
        material_index: Some(index),
        ..Default::default()
    };
    let valid = factor
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value));
    if !valid {
        style.material_invalid = true;
        return style;
    }
    style.color = EntityColor::Explicit([
        linear_to_srgb_byte(factor[0]),
        linear_to_srgb_byte(factor[1]),
        linear_to_srgb_byte(factor[2]),
    ]);
    style.factor = Some([factor[0], factor[1], factor[2]]);
    style.transparency = match material.alpha_mode() {
        AlphaMode::Blend if factor[3] < 1.0 => EntityTransparency::Explicit(factor[3]),
        AlphaMode::Mask => {
            style.alpha_mask_not_imported = true;
            EntityTransparency::Explicit(1.0)
        }
        // `Opaque`, and `Blend` at full alpha, are explicitly opaque.
        AlphaMode::Opaque | AlphaMode::Blend => EntityTransparency::Explicit(1.0),
    };
    style
}

/// Linear glTF component -> the sRGB byte `cad-db` stores (IEC 61966-2-1).
pub(crate) fn linear_to_srgb_byte(linear: f32) -> u8 {
    let clamped = linear.clamp(0.0, 1.0) as f64;
    let encoded = if clamped <= 0.003_130_8 {
        12.92 * clamped
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

fn check_mesh_limits(limits: &ImportLimits, triangles: usize) -> CadResult<()> {
    if let Some(max) = limits.max_mesh_faces {
        if triangles > max {
            return Err(CadError::InvalidInput(format!(
                "a glTF primitive has {triangles} triangle(s), over the {max} face import limit"
            )));
        }
    }
    let indices = triangles.saturating_mul(3);
    if let Some(max) = limits.max_mesh_face_indices {
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "a glTF primitive has {indices} face corner indices, over the {max} face-index \
                 import limit"
            )));
        }
    }
    Ok(())
}
