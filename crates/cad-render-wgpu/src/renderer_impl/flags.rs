use super::*;

impl Renderer {
    /// (Re)create the flags texture, bind group and control buffer so the table
    /// covers at least `key_count` keys.
    ///
    /// The table is one texel per key, packed into rows of at most the adapter's
    /// `max_texture_dimension_2d`; the shader addresses a key as
    /// `(key % width, key / width)`.
    pub(super) fn ensure_flags(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key_count: u32,
    ) {
        let Some(layout) = self.flags_layout.as_ref() else {
            return;
        };
        if self.flags_ctl_buffer.is_none() {
            self.flags_ctl_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cad-flags-ctl"),
                size: u64::from(FLAGS_CTL_BYTES),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let ctl_buffer = self
            .flags_ctl_buffer
            .as_ref()
            .expect("flags control buffer created above");
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        let needed = key_count.max(1);
        let width = flags_table_width(needed, max_dim);
        let height = needed.div_ceil(width).max(1);
        let recreate = self.flags_width != width
            || self
                .flags_texture
                .as_ref()
                .is_none_or(|texture| texture.height() < height);
        if recreate {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cad-flags-table"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FLAGS_FORMAT,
                // COPY_DST so a flags update writes only this table.
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.flags_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cad-flags-bind"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: ctl_buffer.as_entire_binding(),
                    },
                ],
            }));
            self.flags_texture = Some(texture);
            self.flags_view = Some(view);
            self.flags_width = width;
            // The CPU mirror is *preserved* across a recreate and fully
            // re-uploaded into the new row layout below, so growing the key
            // space (an incremental commit) never clears previously-set bits.
            // Only `reset_pick_keys` (a full scene reset) clears the mirror.
            self.flags_table_dirty = true;
        }
        // Keep the CPU mirror at least as long as the addressed texel space; the
        // texture can be reused while the key space grows inside one row.
        let texels = width as usize * height as usize;
        if self.flags_bits.len() < texels {
            self.flags_bits.resize(texels, 0);
        }
        if self.flags_table_dirty {
            self.upload_flags_table(queue, width, height);
        }
        self.write_flags_ctl(queue);
    }

    /// Upload the whole flags table from the CPU mirror.
    ///
    /// Runs once after a table (re)creation or a scene reset — not per frame — so
    /// a stale GPU table can never keep a retired key's bits.
    pub(super) fn upload_flags_table(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        let Some(texture) = self.flags_texture.as_ref() else {
            return;
        };
        let row_bytes = width as usize * 4;
        let aligned = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let mut staging = vec![0u8; aligned * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let bits = self
                    .flags_bits
                    .get(y * width as usize + x)
                    .copied()
                    .unwrap_or(0);
                let slot = y * aligned + x * 4;
                staging[slot] = (bits & 0xFF) as u8;
                staging[slot + 1] = ((bits >> 8) & 0xFF) as u8;
                staging[slot + 2] = ((bits >> 16) & 0xFF) as u8;
                staging[slot + 3] = ((bits >> 24) & 0xFF) as u8;
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &staging,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned as u32),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.flags_upload_bytes += staging.len() as u64;
        self.flags_upload_count += 1;
        self.flags_table_dirty = false;
    }

    /// Upload the 16-byte control uniform (`enabled` + row width).
    pub(super) fn write_flags_ctl(&mut self, queue: &wgpu::Queue) {
        let Some(buffer) = self.flags_ctl_buffer.as_ref() else {
            return;
        };
        let ctl: [u32; 4] = [u32::from(self.flags_enabled), self.flags_width.max(1), 0, 0];
        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&ctl));
    }

    /// Whether the color shaders apply the flags tint. Off by default, so the
    /// existing highlight-overlay batches keep working with no double-highlight.
    pub fn selection_flags_enabled(&self) -> bool {
        self.flags_enabled
    }

    /// Turn the flag-driven tint on or off. The toggle is a 16-byte uniform write
    /// to the flags control buffer; it never touches the static streams or the
    /// per-batch uniforms.
    pub fn set_selection_flags_enabled(&mut self, enabled: bool) {
        self.flags_enabled = enabled;
        // Toggling the tint changes the output: drop any completed progressive
        // frame so the next render redraws instead of reusing the cached pixels.
        self.progressive_frame = None;
        let queue = self.queue.clone();
        if let Some(queue) = queue.as_ref() {
            self.write_flags_ctl(queue);
            self.flags_upload_bytes += u64::from(FLAGS_CTL_BYTES);
            self.flags_upload_count += 1;
        }
    }

    /// Set the [`FLAG_SELECTED`]/[`FLAG_HOVERED`]/[`FLAG_HIDDEN`] bits of one
    /// pick key.
    ///
    /// Uploads **only** the flags table row that contains the key (plus the
    /// 16-byte control uniform); the static instance/merged streams, the batch
    /// buffers and the per-batch uniforms are untouched, which is what the
    /// "a flag update uploads only the flags buffer" fence asserts.
    pub fn set_pick_key_flags(&mut self, key: u32, bits: u32) -> CadResult<()> {
        if key == PICK_KEY_NONE || key > MAX_PICK_KEYS {
            return Err(CadError::InvalidInput(format!(
                "pick key {key} is outside the flags table (1..={MAX_PICK_KEYS})"
            )));
        }
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
        let needed = self.pick_table.max_key().max(key) + 1;
        self.ensure_flags(&device, &queue, needed);
        let Some(texture) = self.flags_texture.as_ref() else {
            return Err(CadError::GpuFailure("flags table unavailable".into()));
        };
        let width = self.flags_width.max(1);
        let row = key / width;
        if self.flags_bits.len() <= key as usize {
            self.flags_bits.resize(key as usize + 1, 0);
        }
        self.flags_bits[key as usize] = bits;
        // Upload the whole row that contains `key`: it is the smallest
        // `copy`-aligned unit the API accepts, and it keeps the update independent
        // of the table's height.
        let row_bytes = width as usize * 4;
        let aligned = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let mut staging = vec![0u8; aligned];
        let base = row as usize * width as usize;
        for x in 0..width as usize {
            let texel = self.flags_bits[base + x];
            staging[x * 4] = (texel & 0xFF) as u8;
            staging[x * 4 + 1] = ((texel >> 8) & 0xFF) as u8;
            staging[x * 4 + 2] = ((texel >> 16) & 0xFF) as u8;
            staging[x * 4 + 3] = ((texel >> 24) & 0xFF) as u8;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: row, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &staging,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned as u32),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.flags_upload_bytes += aligned as u64;
        self.flags_upload_count += 1;
        // A flag change alters the shaded output, so a *completed* progressive
        // frame must not be reused: `render` with an unchanged frame key would
        // otherwise draw nothing and keep the pre-flag pixels (a selection would
        // not tint until the camera moved). Dropping the cached plan forces the
        // next render to redraw from the resident buffers — no geometry re-upload.
        self.progressive_frame = None;
        Ok(())
    }

    /// The CPU mirror of one key's flag bits (0 when unset).
    pub fn pick_key_flags(&self, key: u32) -> u32 {
        self.flags_bits.get(key as usize).copied().unwrap_or(0)
    }

    /// Set the whole-entity [`FLAG_SELECTED`]/[`FLAG_HOVERED`]/[`FLAG_HIDDEN`]
    /// bits for a list of entity references.
    ///
    /// Each `(SelectionRef, bits)` is resolved to the pick keys of the resident
    /// whole-entity batches whose sources name that entity (see
    /// [`PickTable::keys_for_source`]), so an entity reached through an INSERT
    /// instance tints exactly that placement, and a multi-source merged batch is
    /// tinted as a whole (documented batch-granularity limit: the merged run does
    /// not record which fragment range belongs to which source). Bits from
    /// several refs that land on the same key are OR-ed.
    ///
    /// Only the rows whose bits changed since the previous call are uploaded, and
    /// rows that fell out of the list are cleared, so a selection change is a
    /// flags-table update — never a geometry re-upload.
    ///
    /// The call replaces the whole entity-flag set: a key set by a previous
    /// `set_entity_flags` but absent here is cleared to `0`. `set_pick_key_flags`
    /// writes are only overwritten for keys this call actually manages.
    ///
    /// The returned [`EntityFlagReport`] names the refs that resolved to **no**
    /// resident key: the caller must surface them (a base-excluded entity on a
    /// hidden/frozen layer, or geometry that is not resident), never treat the
    /// update as a silent success.
    ///
    /// The request is retained: a later pick-table rebuild (a base change or an
    /// image re-upload) re-resolves it against the renumbered keys, so the bits
    /// never stay on a retired key.
    pub fn set_entity_flags(
        &mut self,
        flags: &[(SelectionRef, u32)],
    ) -> CadResult<EntityFlagReport> {
        self.entity_flags_request = flags.to_vec();
        self.apply_entity_flags()
    }

    /// Resolve the retained entity-flag request against the current pick table.
    ///
    /// Called by [`Self::set_entity_flags`] and again after a pick-table rebuild
    /// (see [`Self::rebuild_pick_table`]), so the bits always land on the
    /// **current** key of each requested entity rather than on a key from a
    /// previous upload that the renumbering retired.
    pub(super) fn apply_entity_flags(&mut self) -> CadResult<EntityFlagReport> {
        let request = self.entity_flags_request.clone();
        // The identity index is built lazily (a scene that never issues an
        // entity-flag request never needs it): make sure it exists before the
        // first query, so a request never silently resolves to "no key".
        if !request.is_empty() {
            self.pick_table.ensure_source_index();
        }
        let mut desired: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        let mut unmatched: Vec<SelectionRef> = Vec::new();
        for (reference, bits) in &request {
            let keys = self.pick_table.keys_for_source(reference);
            if keys.is_empty() {
                unmatched.push(reference.clone());
            }
            for key in keys {
                *desired.entry(key).or_insert(0) |= *bits;
            }
        }
        let stale: Vec<u32> = self
            .entity_flag_keys
            .iter()
            .copied()
            .filter(|key| !desired.contains_key(key))
            .collect();
        for key in stale {
            self.set_pick_key_flags(key, 0)?;
        }
        for (&key, &bits) in &desired {
            if self.pick_key_flags(key) != bits {
                self.set_pick_key_flags(key, bits)?;
            }
        }
        self.entity_flag_keys = desired.keys().copied().collect();
        Ok(EntityFlagReport {
            matched: desired.len(),
            unmatched,
        })
    }

    /// The keys currently carrying an entity-level flag, in ascending order.
    ///
    /// Diagnostic/test surface: it is the diff base of [`Self::set_entity_flags`].
    pub fn entity_flag_keys(&self) -> Vec<u32> {
        self.entity_flag_keys.iter().copied().collect()
    }

    /// Bytes uploaded by flags updates since init. A flags update must not grow
    /// [`Renderer::instance_stream_uploads`] or [`Renderer::mesh_chunk_uploads`].
    pub fn flags_upload_bytes(&self) -> u64 {
        self.flags_upload_bytes
    }

    /// Number of flags updates that reached the GPU.
    pub fn flags_upload_count(&self) -> u64 {
        self.flags_upload_count
    }

    /// The detected `EntityId` truncation collisions of the resident scene.
    ///
    /// Two distinct entities whose low 32 bits agree. Resolution is still exact
    /// (the pick key is a renderer key, not the truncated value), so this is a
    /// *reported* hazard for any 32-bit-only consumer rather than a mis-pick.
    pub fn pick_collisions(&self) -> &[PickCollision] {
        self.pick_table.collisions()
    }

    /// The renderer pick key of resident batch `index`, if any.
    pub fn pick_key_for_batch(&self, index: usize) -> Option<u32> {
        self.batches.get(index).map(|batch| batch.pick_key)
    }

    /// The renderer pick key of resident image `index`, if any. Diagnostic/test.
    pub fn pick_key_for_image(&self, index: usize) -> Option<u32> {
        self.images.get(index).map(|image| image.pick_key)
    }

    /// Number of live entries in the pick table.
    pub fn pick_table_len(&self) -> usize {
        self.pick_table.len()
    }

    /// Diagnostic: cumulative **full** pick-table rebuilds since init.
    ///
    /// A commit that keeps the resident batch prefix (an append/suffix overlay
    /// commit) must extend the table incrementally and leave this flat; only a
    /// base change, an image re-upload, a device rebuild or a full clear bumps
    /// it. Purely observational: it changes no pick semantics.
    pub fn pick_table_full_rebuilds(&self) -> u64 {
        self.pick_table_full_rebuilds
    }

    /// Diagnostic: cumulative incremental pick-table suffix replacements since
    /// init (the complement of [`Self::pick_table_full_rebuilds`]).
    pub fn pick_table_incremental_extends(&self) -> u64 {
        self.pick_table_incremental_extends
    }
}

/// The row width of the flags table for a key space of `needed` keys.
///
/// Rounded up to a power of two (capped at the adapter's `max_dim`) so the width
/// only changes at bucket boundaries. An incremental commit that grows the key
/// space *inside* the current bucket therefore never recreates the table, and a
/// recreation that does happen preserves the CPU mirror (see `ensure_flags`), so
/// previously-set flag bits survive either way. Without bucketing, the width
/// tracked `needed` exactly and every added batch recreated (and zeroed) the
/// table.
pub(super) fn flags_table_width(needed: u32, max_dim: u32) -> u32 {
    needed.max(1).next_power_of_two().min(max_dim.max(1))
}
