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
        let texture = allocation_guard(device, || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cad-image-texture"),
                size: wgpu::Extent3d {
                    width: decoded.width,
                    height: decoded.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        })?;
        // `write_texture` needs the row stride padded to the 256-byte copy
        // alignment; the source rows are tightly packed, so copy through a
        // padded scratch buffer only when the stride is not already aligned.
        let row_bytes = decoded.width as usize * 4;
        let aligned = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded;
        let data: &[u8] = if aligned == row_bytes {
            decoded.rgba.as_ref()
        } else {
            let mut buffer = vec![0u8; aligned * decoded.height as usize];
            for row in 0..decoded.height as usize {
                buffer[row * aligned..row * aligned + row_bytes]
                    .copy_from_slice(&decoded.rgba[row * row_bytes..(row + 1) * row_bytes]);
            }
            padded = buffer;
            &padded
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned as u32),
                rows_per_image: Some(decoded.height),
            },
            wgpu::Extent3d {
                width: decoded.width,
                height: decoded.height,
                depth_or_array_layers: 1,
            },
        );
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
            upload_bytes: aligned * decoded.height as usize,
            decoded: Arc::clone(decoded),
            texture_charge,
        })
    }
}
