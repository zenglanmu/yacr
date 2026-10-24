use super::*;

/// A device-resident texture for one [`ResourceKey`], shared by every resident
/// image quad that references it.
///
/// [`Renderer`] caches one per key (see [`Renderer::upload_images`]) so `N`
/// entities referencing the same resource allocate a single texture and bind
/// group rather than `N` copies.
pub(crate) struct GpuTexture {
    /// The uploaded texture. Kept alive both for the bind group and so a new
    /// quad can clone a handle into its [`GpuImage`].
    pub(crate) texture: wgpu::Texture,
    /// Group 2 bind group (texture view + shared sampler), shared by every quad
    /// that samples this key.
    pub(crate) bind_group: wgpu::BindGroup,
    /// Padded RGBA bytes actually uploaded: `height` rows of `width * 4` bytes
    /// each rounded up to wgpu's 256-byte copy alignment. Charged once per key
    /// against the frame byte budget.
    pub(crate) upload_bytes: usize,
    /// The decoded image this texture was uploaded from, held by identity so the
    /// cache can tell whether a reused [`ResourceKey`] still refers to the same
    /// bytes. A new [`DecodedImageCache`] produces a new `Arc`, so a different
    /// document's `logo.png` is re-uploaded instead of sampling the previous
    /// drawing's texture. Holding the `Arc` (rather than a raw pointer) keeps the
    /// allocation alive, so `Arc::ptr_eq` cannot report a false match after the
    /// old image is freed.
    pub(crate) decoded: Arc<DecodedImage>,
    /// GPU memory charge of `texture` (released with it). A quad that samples this
    /// texture holds a handle clone but no second charge: the allocation is
    /// counted once, where it was created.
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) texture_charge: GpuCharge,
}

/// One uploaded raster image: a textured quad plus the bind groups needed to
/// draw it. Owned by [`Renderer`] and rebuilt by [`Renderer::upload_images`].
pub(crate) struct GpuImage {
    /// The resource key this quad samples; quads sharing a key share a texture.
    pub(crate) resource: ResourceKey,
    /// World-space quad vertices (position + uv), indexed by `index_buffer`.
    pub(crate) vertex_buffer: wgpu::Buffer,
    pub(crate) index_buffer: wgpu::Buffer,
    pub(crate) index_count: u32,
    pub(crate) vertex_count: u32,
    /// Group 2: the sampled texture and its sampler (shared per resource key).
    pub(crate) texture_bind_group: wgpu::BindGroup,
    /// A handle to the shared texture, kept alive for as long as the texture view
    /// bound in `texture_bind_group` is used.
    pub(crate) _texture: wgpu::Texture,
    /// Padded texture bytes for this quad's resource key, charged to the frame
    /// byte budget once per key (not once per quad).
    pub(crate) texture_bytes: usize,
    /// Constant per-image alpha, already sanitized at upload.
    pub(crate) alpha: f32,
    /// Paint order relative to sibling images (larger is later/on top). Full
    /// interleaving with geometry batches is *not* implemented (see
    /// [`Renderer::upload_images`]).
    pub(crate) draw_order: i64,
    /// Exact bytes this quad uploads (vertex + index buffers).
    pub(crate) upload_bytes: usize,
    /// This image's renderer pick key (see [`PickTable`]).
    pub(crate) pick_key: u32,
    /// The full identities this quad can belong to (an `ImageBatch` carries its
    /// own `sources`), preserved for the CPU-side pick table.
    pub(crate) sources: Vec<SelectionRef>,
    /// GPU memory charge of this quad's vertex buffer.
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) vertex_charge: GpuCharge,
    /// GPU memory charge of this quad's index buffer.
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) index_charge: GpuCharge,
}
