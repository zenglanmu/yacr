use super::*;

/// Pack the per-image display transfer into the two `key_flags` words every
/// batch uniform already has, so the uniform layout does not change.
///
/// * `z = fade | contrast << 8 | brightness << 16` (each 0..=100, 8 bits);
/// * `w = monochrome | bg_r << 8 | bg_g << 16 | bg_b << 24`, where `bg_*` is the
///   render target's clear colour (the colour `fade` blends toward).
///
/// The image shader decodes them (see `shaders/image.wgsl`); every other
/// pipeline keeps its own meaning for these words (the line AA ramp lives in
/// `.z`), which is why only the image loop calls this.
pub(crate) fn image_key_flags(
    pick_key: u32,
    adjust: ImageAdjust,
    background: [f32; 4],
) -> [u32; 4] {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    [
        pick_key,
        0,
        u32::from(adjust.fade)
            | u32::from(adjust.contrast) << 8
            | u32::from(adjust.brightness) << 16,
        u32::from(adjust.monochrome)
            | channel(background[0]) << 8
            | channel(background[1]) << 16
            | channel(background[2]) << 24,
    ]
}

/// Write one per-image uniform slot per drawn image, starting at `first_slot`,
/// and record each slot in `image_slots`.
///
/// The transform is the camera-only matrix (image vertices are already world
/// space) and the tint is white with the image's alpha. The display transfer
/// rides the same uniform's `key_flags.z`/`.w` words (see [`image_key_flags`]),
/// so the uniform layout is unchanged.
///
/// A free function rather than a `Renderer` method: the frame path also holds
/// immutable borrows of other `Renderer` fields (`device`, `queue`), so only the
/// two mutable arenas are passed in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_image_uniforms(
    images: &[GpuImage],
    draw_images: &[usize],
    image_slots: &mut [u32],
    uniforms: &mut UniformArena,
    camera_matrix: [f32; 16],
    background: [f32; 4],
    light: [f32; 4],
    first_slot: usize,
) {
    for (j, &i) in draw_images.iter().enumerate() {
        let image = &images[i];
        let key_flags = image_key_flags(image.pick_key, image.adjust, background);
        let uniform = uniform_bytes(
            &camera_matrix,
            [1.0, 1.0, 1.0, image.alpha],
            key_flags,
            light,
        );
        let slot = first_slot + j;
        uniforms.write_slot(slot, &uniform);
        image_slots[i] = slot as u32;
    }
}

impl Renderer {
    /// Upload raster image quads from `images`, resolving each `ResourceKey`
    /// through `cache`.
    ///
    /// Every image that resolves to a decodable [`cad_resources::DecodedImage`]
    /// becomes a textured quad: the RGBA8 bytes go to an `Rgba8UnormSrgb`
    /// texture, a linear clamp-to-edge sampler view/bind group is created, and
    /// `transform` maps the unit-square corners `(0,0),(1,0),(1,1),(0,1)` (UVs
    /// `(0,0),(1,0),(1,1),(0,1)`, two triangles) into world space.
    ///
    /// An image whose resource is absent from the cache (or whose decoded buffer
    /// is malformed) is **skipped**, never drawn as a placeholder, and listed in
    /// [`ImageUploadReport::unresolved`] so the caller can surface
    /// `image.unresolved`. This replaces the resident image set.
    ///
    /// One GPU texture (and bind group) is allocated per unique `ResourceKey`
    /// and shared by every quad that references it, so `N` entities referencing
    /// the same resource upload the texture once. The texture cache is pruned to
    /// the keys of the resident set on each call.
    ///
    /// Reuse is identity-aware rather than key-only: a cached texture is only
    /// reused while `cache.get(key)` yields the *same* [`DecodedImage`] it was
    /// uploaded from. Because the `Renderer` outlives the per-document
    /// [`DecodedImageCache`], a key that reappears with different bytes (for
    /// example two drawings that both reference `logo.png`) rebuilds the texture
    /// instead of sampling the previous drawing's image. Texture bytes are
    /// charged only when a texture is created or rebuilt.
    ///
    /// When a batch carries a `clip` polygon, that surviving convex polygon
    /// (world-space positions with per-vertex texture coordinates) is drawn and
    /// fan-triangulated instead of the unit square; the whole unit square is
    /// drawn only when `clip` is `None`.
    ///
    /// Not implemented, and therefore not claimed: WIPEOUT-style masking and
    /// full draw-order interleaving with geometry batches are not performed —
    /// images draw after opaque/transparent geometry, ordered among themselves
    /// by ascending `draw_order`.
    pub fn upload_images(
        &mut self,
        images: &[ImageBatch],
        cache: &DecodedImageCache,
    ) -> CadResult<ImageUploadReport> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let texture_layout = self
            .image_texture_layout
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let sampler = self
            .image_sampler
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();

        // Every upload starts from "no refusal pending" (see `gpu_memory`). A
        // budget refusal below leaves the resident quads (`self.images`) and their
        // textures rendering; a texture created earlier in this refused call stays
        // in the cache and is pruned by the next successful call (never drawn: no
        // quad references it).
        self.clear_gpu_memory_refusal();
        let mut report = ImageUploadReport::default();
        let mut staged = Vec::with_capacity(images.len());
        // Allocate one renderer pick key per requested image up front, mirroring
        // the batch path (`prepare_upload_batches`): exhaustion is an
        // invalid-input error rather than a silently unpickable key. A skipped
        // (unresolved) image still burns its key; keys are monotonic and never
        // reused within a device generation, so that is only a small waste.
        let mut image_keys = Vec::with_capacity(images.len());
        for _ in 0..images.len() {
            let key = self.pick_key_counter.fetch_add(1, Ordering::Relaxed);
            if key == PICK_KEY_NONE || key > MAX_PICK_KEYS {
                return Err(CadError::InvalidInput(
                    "renderer pick key space exhausted (too many images uploaded \
                     without a full scene clear)"
                        .into(),
                ));
            }
            image_keys.push(key);
        }
        // Resource keys still referenced after this call; textures for any other
        // key are dropped so the cache tracks the resident set, not history.
        let mut live: std::collections::HashSet<ResourceKey> = std::collections::HashSet::new();
        for (index, batch) in images.iter().enumerate() {
            let Some(decoded) = cache.get(&batch.resource) else {
                report.unresolved.push(batch.resource.clone());
                continue;
            };
            let expected = decoded.width as usize * decoded.height as usize * 4;
            if decoded.width == 0 || decoded.height == 0 || decoded.rgba.len() != expected {
                // A malformed decoded buffer is neither drawn nor hidden.
                report.unresolved.push(batch.resource.clone());
                continue;
            }
            // One texture per key: reuse the cached texture when an earlier batch
            // (or a previous call) already uploaded this resource, so N entities
            // referencing the same key allocate a single texture/bind group.
            //
            // Reuse is identity-aware, not key-only: a `ResourceKey` can outlive
            // the bytes it names (each document open builds a fresh
            // `DecodedImageCache` while the `Renderer` persists). Rebuild whenever
            // the cached texture was uploaded from a different decoded image, so
            // two drawings that both reference e.g. `logo.png` with different
            // bytes cannot silently share the first drawing's texture. Within one
            // cache `get` clones the same `Arc`, so sibling batches still reuse.
            let reusable = self
                .image_textures
                .get(&batch.resource)
                .is_some_and(|existing| Arc::ptr_eq(&existing.decoded, &decoded));
            if !reusable {
                // The texture is the expensive part of an image: gate it before the
                // allocation (the refusal is recorded, so a `CadResult` caller can
                // still read the structured facts through
                // `Renderer::gpu_budget_refusal`).
                let requested = u64::from(decoded.width) * u64::from(decoded.height) * 4;
                self.admit_gpu_memory(&[(GpuMemoryCategory::ImageTexture, requested)])?;
                let created = Self::build_texture(
                    &device,
                    &queue,
                    &texture_layout,
                    &sampler,
                    &decoded,
                    &self.gpu_memory,
                )
                .map_err(|error| self.image_allocation_error(&batch.resource, error))?;
                report.bytes += created.upload_bytes as u64;
                self.image_textures.insert(batch.resource.clone(), created);
            }
            let texture = &self.image_textures[&batch.resource];
            let (vertices, indices) = image_quad_vertices(batch);
            self.admit_gpu_memory(&[
                (GpuMemoryCategory::Vertex, vertices.len() as u64),
                (GpuMemoryCategory::Index, (indices.len() * 4) as u64),
            ])?;
            let vertex_buffer = Self::vertex_buffer(&device, Some(&queue), &vertices);
            let index_buffer = Self::index_buffer(&device, Some(&queue), &indices);
            let vertex_charge = self.gpu_charge(GpuMemoryCategory::Vertex, vertex_buffer.size());
            let index_charge = self.gpu_charge(GpuMemoryCategory::Index, index_buffer.size());
            let quad_bytes = vertices.len() + indices.len() * 4;
            report.bytes += quad_bytes as u64;
            report.uploaded += 1;
            live.insert(batch.resource.clone());
            staged.push(GpuImage {
                resource: batch.resource.clone(),
                vertex_buffer,
                index_buffer,
                index_count: indices.len() as u32,
                vertex_count: (vertices.len() / 20) as u32,
                texture_bind_group: texture.bind_group.clone(),
                _texture: texture.texture.clone(),
                texture_bytes: texture.upload_bytes,
                alpha: clamp_alpha(batch.alpha),
                draw_order: batch.draw_order,
                adjust: batch.adjust,
                upload_bytes: quad_bytes,
                pick_key: image_keys[index],
                sources: batch.sources.clone(),
                vertex_charge,
                index_charge,
            });
        }
        self.image_textures.retain(|key, _| live.contains(key));
        self.images = staged;
        // The resident image set was replaced wholesale (fresh keys per call), so
        // this is a full rebuild, never an incremental suffix replacement.
        self.rebuild_pick_table(None)?;
        // The resident image set was replaced wholesale (fresh keys per call, a
        // full rebuild — never an incremental suffix), and image quads are drawn
        // scene data that change the draw-cost profile, so the page cost knowledge
        // is stale and a reset is justified. In practice the runtime only calls
        // this on a base change (`runtime.rs`), which already reset via
        // `commit_upload`; the reset here keeps the contract true for any caller.
        self.reset_pacing();
        Ok(report)
    }

    /// Resolve and upload the mesh material textures of a scene (ADR-0038
    /// D9/D10, ADR-0039 D6).
    ///
    /// `keys` is the deduplicated texture-key set the resident batches reference
    /// (`cad_scene::mesh_texture_keys` / `SceneCache::mesh_texture_keys`), which
    /// includes the base-colour key **and** the four PBR slots. Each key is
    /// resolved through the **same** decoded-image cache the raster images use and
    /// uploaded with the image pipeline's texture layout
    /// ([`Self::image_texture_layout`]), so mesh textures and image quads share one
    /// allocation/decode path.
    ///
    /// Unlike a raster image quad, every mesh texture is uploaded with a
    /// **CPU-generated mip chain** (`texture_mip::generate_mip_levels`, one
    /// `write_texture` per level) and is sampled by the dedicated
    /// `mesh_sampler` (trilinear mip filtering, 16× anisotropy). Without it
    /// a 2048² albedo minified onto a few hundred pixels sampled level 0 alone and
    /// aliased into striations.
    ///
    /// Texel format follows the key's **role**, read from the resident batches:
    ///
    /// * base colour and emissive are colour → `Rgba8UnormSrgb` (linear on sample),
    ///   mips averaged in **linear light** and re-encoded;
    /// * metallic-roughness, normal and occlusion are data → `Rgba8Unorm`. Decoding
    ///   a normal map through sRGB would break the tangent-space normal. Its mips
    ///   are averaged and then **renormalised**; the two scalar maps get a plain
    ///   box average.
    ///
    /// A key used in both roles is uploaded once per role (two textures); that is
    /// rare and keeps each sample correct rather than picking one convention. A
    /// key used as both a normal map and a scalar map gets the `Linear` mip rule
    /// (renormalising a scalar map would corrupt it).
    ///
    /// The budget request and [`ImageUploadReport::bytes`] both count the **whole**
    /// chain, so a mip-chained upload is ~1/3 larger than level 0 and a budget
    /// refusal cannot be dodged by ignoring the extra levels.
    ///
    /// After the residency update, one bind group is created per distinct
    /// five-slot channel set referenced by a resident PBR batch
    /// ([`Self::pbr_bind_groups`]). A slot whose key is unresolved binds its
    /// **identity** texture (channel disabled) and is named in
    /// [`ImageUploadReport::unresolved`]; it is reported again at draw time through
    /// [`Renderer::mesh_texture_fallbacks`]. A batch whose channel set has not been
    /// resolved yet draws with the all-identity bind group.
    ///
    /// This is a texture-residency call only: it creates no draw item and no pick
    /// key (a mesh texture is never pickable on its own — picking resolves the
    /// batch), so it does not touch the pick table, the draw plan or pacing.
    pub fn upload_mesh_textures(
        &mut self,
        keys: &[ResourceKey],
        cache: &DecodedImageCache,
    ) -> CadResult<ImageUploadReport> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let texture_layout = self
            .image_texture_layout
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        // The **mesh** sampler, not the raster-image one: mesh material channels
        // are uploaded with a CPU-generated mip chain, so they need trilinear mip
        // filtering (and anisotropy). `image_sampler` keeps its one-level,
        // `Nearest`-mip behaviour so CAD raster image quads are unchanged.
        let sampler = self
            .mesh_sampler
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();

        self.clear_gpu_memory_refusal();
        let mut report = ImageUploadReport::default();
        // Which role each key plays, derived from the resident batches (the scene
        // already sanitised the keys, so a key is only here if a batch declares it).
        let mut colour_role: std::collections::HashSet<ResourceKey> =
            std::collections::HashSet::new();
        let mut data_role: std::collections::HashSet<ResourceKey> =
            std::collections::HashSet::new();
        // The mip filtering rule of each **data** key. A normal map needs
        // renormalisation; metallic-roughness and occlusion are plain linear
        // averages. A key used in both roles gets `Linear`: renormalising a
        // scalar map would corrupt it, and the shared-key case is already
        // ambiguous data rather than a supported configuration.
        let mut data_mip_role: HashMap<ResourceKey, MipRole> = HashMap::new();
        for batch in &self.batches {
            if let Some(key) = &batch.texture {
                colour_role.insert(key.clone());
            }
            if let Some(pbr) = &batch.pbr {
                if let Some(key) = &pbr.emissive_texture {
                    colour_role.insert(key.clone());
                }
                if let Some(key) = &pbr.normal {
                    data_mip_role.entry(key.clone()).or_insert(MipRole::Normal);
                }
                for key in [&pbr.metallic_roughness, &pbr.occlusion]
                    .into_iter()
                    .flatten()
                {
                    data_mip_role.insert(key.clone(), MipRole::Linear);
                }
                for key in [&pbr.metallic_roughness, &pbr.normal, &pbr.occlusion]
                    .into_iter()
                    .flatten()
                {
                    data_role.insert(key.clone());
                }
            }
        }
        let mut live_colour: std::collections::HashSet<ResourceKey> =
            std::collections::HashSet::new();
        let mut live_data: std::collections::HashSet<ResourceKey> =
            std::collections::HashSet::new();
        // A key with no batch role can only come from the caller's list; treat it
        // as a base-colour key (the pre-ADR-0039 contract).
        let mut unresolved: std::collections::BTreeSet<ResourceKey> =
            std::collections::BTreeSet::new();
        let mut uploaded_any = 0usize;
        for key in keys {
            let wants_colour = colour_role.contains(key) || !data_role.contains(key);
            let wants_data = data_role.contains(key);
            let Some(decoded) = cache.get(key) else {
                unresolved.insert(key.clone());
                continue;
            };
            let expected = decoded.width as usize * decoded.height as usize * 4;
            if decoded.width == 0 || decoded.height == 0 || decoded.rgba.len() != expected {
                unresolved.insert(key.clone());
                continue;
            }
            if wants_colour {
                let reusable = self
                    .mesh_textures
                    .get(key)
                    .is_some_and(|existing| Arc::ptr_eq(&existing.decoded, &decoded));
                if !reusable {
                    // The request is the **whole** chain (level 0 plus every
                    // generated level), not just level 0: a budget refusal that
                    // ignored the mips would admit a texture the ledger then
                    // charges more for.
                    let requested = mipped_rgba8_bytes(decoded.width, decoded.height);
                    self.admit_gpu_memory(&[(GpuMemoryCategory::ImageTexture, requested)])?;
                    let created = Self::build_texture_with_format(
                        &device,
                        &queue,
                        &texture_layout,
                        &sampler,
                        &decoded,
                        &self.gpu_memory,
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        Some(MipRole::SrgbColour),
                    )
                    .map_err(|error| self.image_allocation_error(key, error))?;
                    report.bytes += created.upload_bytes as u64;
                    self.mesh_textures.insert(key.clone(), created);
                }
                live_colour.insert(key.clone());
                uploaded_any += 1;
            }
            if wants_data {
                let reusable = self
                    .pbr_data_textures
                    .get(key)
                    .is_some_and(|existing| Arc::ptr_eq(&existing.decoded, &decoded));
                if !reusable {
                    let role = data_mip_role.get(key).copied().unwrap_or(MipRole::Linear);
                    let requested = mipped_rgba8_bytes(decoded.width, decoded.height);
                    self.admit_gpu_memory(&[(GpuMemoryCategory::ImageTexture, requested)])?;
                    let created = Self::build_texture_with_format(
                        &device,
                        &queue,
                        &texture_layout,
                        &sampler,
                        &decoded,
                        &self.gpu_memory,
                        wgpu::TextureFormat::Rgba8Unorm,
                        Some(role),
                    )
                    .map_err(|error| self.image_allocation_error(key, error))?;
                    report.bytes += created.upload_bytes as u64;
                    self.pbr_data_textures.insert(key.clone(), created);
                }
                live_data.insert(key.clone());
                uploaded_any += 1;
            }
        }
        report.unresolved = unresolved.into_iter().collect();
        report.uploaded = uploaded_any;
        // Prune textures no batch references any more: the maps track the resident
        // set, not history.
        self.mesh_textures
            .retain(|key, _| live_colour.contains(key));
        self.pbr_data_textures
            .retain(|key, _| live_data.contains(key));

        // Rebuild one group-2 bind group per distinct PBR channel set. A missing
        // slot binds its identity texture (channel disabled), never a placeholder.
        self.pbr_bind_groups.clear();
        if let (Some(layout), Some(identities), Some(_)) = (
            self.pbr_texture_layout.as_ref(),
            self.pbr_identities.as_ref(),
            self.pbr_identity_bind_group.as_ref(),
        ) {
            for batch in &self.batches {
                if batch.pbr.is_none() {
                    continue;
                }
                let set = PbrTextureSet::of(batch);
                if self.pbr_bind_groups.contains_key(&set) {
                    continue;
                }
                let bind_group = Self::build_pbr_bind_group(
                    &device,
                    layout,
                    &sampler,
                    &set,
                    identities,
                    &self.mesh_textures,
                    &self.pbr_data_textures,
                );
                self.pbr_bind_groups.insert(set, bind_group);
            }
        }
        Ok(report)
    }

    /// One group-2 bind group for a PBR channel set.
    ///
    /// Colour slots (`base`, `emissive`) resolve through the sRGB mesh textures,
    /// data slots (`metallic_roughness`, `normal`, `occlusion`) through the linear
    /// PBR data textures. A missing key falls back to the stored identity view for
    /// that slot, so the channel contributes exactly 1 (or "no perturbation").
    #[allow(clippy::too_many_arguments)]
    fn build_pbr_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        set: &PbrTextureSet,
        identities: &PbrIdentities,
        colour: &HashMap<ResourceKey, GpuTexture>,
        data: &HashMap<ResourceKey, GpuTexture>,
    ) -> wgpu::BindGroup {
        // A fresh view per bound texture: `GpuTexture` keeps its single-texture
        // bind group, not a view, and composing one view per slot on a base change
        // is cheaper than threading a second handle through the upload struct.
        let view_of = |texture: &GpuTexture| {
            texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let colour_view = |key: &Option<ResourceKey>, identity: &wgpu::TextureView| {
            key.as_ref()
                .and_then(|key| colour.get(key))
                .map_or_else(|| identity.clone(), view_of)
        };
        let data_view = |key: &Option<ResourceKey>, identity: &wgpu::TextureView| {
            key.as_ref()
                .and_then(|key| data.get(key))
                .map_or_else(|| identity.clone(), view_of)
        };
        let views = [
            colour_view(&set.base, &identities.white),
            data_view(&set.metallic_roughness, &identities.white),
            data_view(&set.normal, &identities.flat_normal),
            colour_view(&set.emissive, &identities.black),
            data_view(&set.occlusion, &identities.white),
        ];
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-pbr-texture-bind"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&views[4]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Monotonic count of PBR batches whose tangent frame could not be generated
    /// from degenerate UVs (ADR-0039 D6).
    ///
    /// Such a batch still draws (scalar factors are valid), but its normal-map
    /// channel is **disabled** rather than evaluated against an invented tangent.
    /// A non-zero value is the [`codes::RENDER_PBR_TANGENT_UNAVAILABLE`]
    /// (`render.pbr.tangent_unavailable`) evidence; the counter only ever
    /// increases and is never reset by a camera or scene change.
    pub fn pbr_tangent_unavailable(&self) -> u64 {
        self.pbr_tangent_unavailable
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Texture keys a drawn batch wanted but could not sample in the last frame.
    ///
    /// Empty in the normal case. A non-empty list means those batches were drawn
    /// with the untextured pipeline (batch/vertex colour) because the key was not
    /// resident — a decode failure, a refused upload, or an upload that has not
    /// run yet. The renderer never binds a placeholder texture, so this is the
    /// only way that degradation is observable.
    pub fn mesh_texture_fallbacks(&self) -> &[ResourceKey] {
        &self.mesh_texture_fallbacks
    }

    /// The sampler bound at group 2 of the PBR and textured-mesh pipelines:
    /// trilinear mip filtering with 16× anisotropy.
    ///
    /// Crate-visible so the GPU contract in `mesh_pbr_gpu_tests` can sample a
    /// minified texture with the **same** sampler the production bind groups use
    /// (a test-local sampler would not prove anything about the product path).
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn mesh_sampler(&self) -> Option<&wgpu::Sampler> {
        self.mesh_sampler.as_ref()
    }

    /// Turn one failed image allocation into the crate's `CadError` **and** the
    /// recorded explicit refusal.
    ///
    /// A device out-of-memory report is classified as
    /// [`GpuMemoryRefusalOrigin::DeviceOutOfMemory`](crate::GpuMemoryRefusalOrigin)
    /// (never a generic failure string); any other captured error is reported with
    /// its own message. The image set is unchanged either way.
    fn image_allocation_error(
        &self,
        resource: &ResourceKey,
        error: PulledAllocationError,
    ) -> CadError {
        match error {
            PulledAllocationError::OutOfMemory(message) => {
                let refusal = self.gpu_memory.device_out_of_memory(
                    GpuMemoryCategory::ImageTexture,
                    0,
                    message,
                );
                CadError::GpuFailure(refusal.message().to_string())
            }
            PulledAllocationError::Other(message) => CadError::GpuFailure(format!(
                "uploading image {} failed: {message}",
                resource.as_str()
            )),
        }
    }

    /// Upload one decoded RGBA image into a device texture and bind it to the
    /// shared sampler. The returned texture is cached per resource key and
    /// shared by every quad that references it.
    ///
    /// This is the **raster image quad** path: an `Rgba8UnormSrgb` texture with
    /// no mip chain (`mip_level_count: 1`), sampled by the renderer's
    /// `image_sampler`. Mesh material channels take
    /// [`Self::build_texture_with_format`] with a [`MipRole`].
    ///
    /// Fallible: the texture (and its bind group) are created inside the crate's
    /// allocation guard, so a driver refusal is classified instead of reaching
    /// wgpu's default handler, which aborts the process on an invalid handle.
    pub(super) fn build_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        decoded: &Arc<DecodedImage>,
        ledger: &Arc<GpuMemoryLedger>,
    ) -> Result<GpuTexture, PulledAllocationError> {
        Self::build_texture_with_format(
            device,
            queue,
            texture_layout,
            sampler,
            decoded,
            ledger,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            None,
        )
    }

    /// [`Self::build_texture`] with an explicit texel format and mip role.
    ///
    /// Base-colour and emissive maps are colour and use `Rgba8UnormSrgb`; the
    /// metallic-roughness, normal and occlusion maps are linear data and use
    /// `Rgba8Unorm` (ADR-0039 D6) — sRGB-decoding a normal map would corrupt it.
    ///
    /// `mip_role` decides the CPU mip filtering rule ([`MipRole`]); `None`
    /// creates a one-level texture. When it is `Some`, the texture is created
    /// with the full chain and **every** level is written, so the sampler can
    /// select any of them.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_texture_with_format(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        decoded: &Arc<DecodedImage>,
        ledger: &Arc<GpuMemoryLedger>,
        format: wgpu::TextureFormat,
        mip_role: Option<MipRole>,
    ) -> Result<GpuTexture, PulledAllocationError> {
        // Levels 1..end, generated here on the CPU: the bytes are already
        // decoded, the result is deterministic and no backend-specific render
        // pass (or per-backend variance) is involved.
        let levels = mip_role.map_or_else(Vec::new, |role| {
            generate_mip_levels(decoded.rgba.as_ref(), decoded.width, decoded.height, role)
        });
        // Level 0 plus every generated level; an empty `levels` is exactly one
        // level, so both paths share the same arithmetic.
        let mip_levels = 1 + levels.len() as u32;
        if mip_role.is_some() {
            debug_assert_eq!(mip_levels, mip_level_count(decoded.width, decoded.height));
        }
        let texture = allocation_guard(device, || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cad-image-texture"),
                size: wgpu::Extent3d {
                    width: decoded.width,
                    height: decoded.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mip_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        })?;
        // One `write_texture` per level, each with that level's own size and
        // 256-byte-padded row stride (level 0 included).
        let mut upload_bytes = write_texture_level(
            queue,
            &texture,
            0,
            decoded.width,
            decoded.height,
            decoded.rgba.as_ref(),
        );
        for (index, level) in levels.iter().enumerate() {
            upload_bytes += write_texture_level(
                queue,
                &texture,
                index as u32 + 1,
                level.width,
                level.height,
                &level.rgba,
            );
        }
        // `TextureViewDescriptor::default()` spans **every** mip level
        // (`base_mip_level: 0`, `mip_level_count: None`), so each bind group below
        // exposes the whole chain to the sampler.
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = allocation_guard(device, || {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cad-image-texture-bind"),
                layout: texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        })?;
        let texture_charge = ledger.charge(
            GpuMemoryCategory::ImageTexture,
            gpu_memory::texture_bytes(&texture),
        );
        Ok(GpuTexture {
            texture,
            bind_group,
            // Every level's padded bytes: a mip-chained texture reports (and is
            // charged) more than level 0 alone.
            upload_bytes,
            decoded: Arc::clone(decoded),
            texture_charge,
        })
    }
}
