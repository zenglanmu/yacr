//! Native headless render and paper-space plot operations.

use super::*;

mod camera;

/// Map the CLI-local [`GpuPreference`] onto the renderer's `GpuSelection`.
///
/// Native only: `cad-cli-tools` depends on `cad-render-wgpu` solely on this
/// target (wasm keeps the renderer out of the dependency graph).
#[cfg(not(target_arch = "wasm32"))]
fn render_gpu(invocation: &CliInvocation) -> cad_render_wgpu::GpuSelection {
    match invocation.gpu {
        GpuPreference::Auto => cad_render_wgpu::GpuSelection::Auto,
        GpuPreference::HighPerformance => cad_render_wgpu::GpuSelection::HighPerformance,
        GpuPreference::LowPower => cad_render_wgpu::GpuSelection::LowPower,
    }
}

/// Native fixed-viewport render.
///
/// Imports the drawing through the shared app path, builds the same display
/// representation as `build-representation`, batches it through `SceneCache`,
/// uploads it to a real headless wgpu device, renders one frame, reads it back
/// and optionally writes a PNG.
///
/// Never an empty success: a machine with no adapter fails with
/// `CadError::GpuFailure`, a drawing with no drawable bounds fails with
/// `CadError::InvalidInput`, and the PNG is written only after a frame exists.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_render(invocation: &CliInvocation) -> Result<serde_json::Value, CliError> {
    // Refuse a bad `--view`/`--azimuth`/`--elevation`/`--fov` combination before
    // the import: an option combination is a usage error, not a render failure.
    invocation.requested_view()?;
    let controller = load_document(invocation)?;
    render_document(invocation, &controller)
}

/// The GPU half of [`run_render`] on an already-open document.
///
/// Split out (the caller imports; this renders) so a contract test can drive a
/// document it installed itself: the textured-mesh regression needs a drawing
/// whose texture bytes live in the database, and writing a carrier-owned
/// `.ymodel` from this crate would test the archive instead of the renderer.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn render_document(
    invocation: &CliInvocation,
    controller: &HostController,
) -> Result<serde_json::Value, CliError> {
    use cad_render_wgpu::headless::{create_headless_gpu_with, HeadlessGpu};
    use cad_render_wgpu::{BackendPreference, RenderTarget, Renderer};

    // A 3D view is only well defined for a consistent flag set; the same check
    // `run_render` makes up front, repeated for callers that hand a controller in
    // directly (the contract tests).
    let requested_view = invocation.requested_view()?;

    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let fonts = load_fonts(
        &invocation.fonts,
        &fonts_requested(controller),
        drawing_needs_text(controller),
        drawing_locale(controller),
    )?;
    let (image_cache, image_report) = load_images(invocation, controller)?;
    let mut context = representation_context(controller, fonts.as_ref());
    // The render feeds the GPU directly: packed dash output + compact line
    // batches, identical to the desktop scene path. `build-representation`
    // intentionally keeps the unpacked form so its primitive counts describe
    // fragments, not render batches.
    context = context.with_packed_line_segments();
    // A drawing with no raster images at all keeps the previous context exactly
    // (no texture path); a decoded cache lets an `Image` become a texture and a
    // textured mesh sample its document-owned base colour. `requested` names both
    // sources (file keys and document textures), so a texture-only drawing still
    // attaches the cache (ADR-0038 D10).
    if !image_report.requested.is_empty() {
        context = context.with_images(image_cache.clone());
    }

    // One delta over every model-space entity.
    let mut cache = cad_scene::SceneCache::new(Default::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    };
    let mut batches: u64 = 0;
    let mut vertices: u64 = 0;
    for entity in document.drawing.model_space() {
        // A top-level entity the source baked `invisible` is not display output:
        // the GUI scene path skips it, so the headless render must too.
        if document.drawing.entity_hidden(entity.id) {
            continue;
        }
        let representation = registry.build_expanded(&document.drawing, entity, &context)?;
        // `build_compact` merges adjacent same-style opaque line fragments
        // (capped at 65,536 vertices / 8,192 world units), the fix that took a
        // comparable desktop scene from 20.8M batches to ~97k. Without it a
        // block-reference explosion turns a few-MB DWG into tens of millions of
        // tiny batches that exhaust host memory (~20 GB RSS, OOM-killed).
        let built = cache.build_compact(&representation, context.stamp.clone())?;
        batches = batches.saturating_add(built.added.len() as u64);
        vertices = vertices.saturating_add(
            built
                .added
                .iter()
                .map(|batch| batch.vertices.len() as u64)
                .sum::<u64>(),
        );
        enforce_scene_budget(batches, vertices, invocation)?;
        delta.added.extend(built.added);
        delta.images.extend(built.images);
        delta.masks.extend(built.masks);
    }

    // Fit to what is actually drawn, not `drawing.bounds()`: the database
    // bounds include material the renderer does not draw (text, unplaced or
    // block-definition geometry), which leaves the framed image off-centre and
    // small. `camera::fit_cameras` derives the unchanged 2D plan fit from the
    // scene batches' world-space points (`local_origin + vertex`) and, in the
    // same walk, the 3D extent a `--view`/`--azimuth` frame needs.
    let fitted = camera::fit_cameras(&delta, invocation.render_width, invocation.render_height);
    // Nothing drawable at all is still an explicit refusal before any GPU work,
    // on both camera paths. A scene with no finite vertex z (but drawable XY)
    // keeps a fitted 2D camera and falls back to `enter_3d(None)` in 3D; a
    // conic-only scene has 3D bounds but no 2D vertex fit.
    if fitted.plan.is_none() && fitted.bounds3.is_none() {
        return Err(CadError::InvalidInput("no drawable geometry to render".into()).into());
    }
    let width = invocation.render_width;
    let height = invocation.render_height;

    let HeadlessGpu {
        device,
        queue,
        adapter,
    } = create_headless_gpu_with(BackendPreference::Auto, render_gpu(invocation))?;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // `--gpu-budget-mb` overrides the renderer's own `YACR_GPU_BUDGET_MB` /
    // default policy (flag > env > default); see `gpu_memory`.
    apply_resolved_gpu_budget(&mut renderer, invocation);
    // A large drawing is tens of thousands of small draw calls; a CPU software
    // adapter can legitimately exceed the interactive 1 s submission bound.
    // Headless evidence waits longer instead of misreporting a device loss.
    renderer.set_poll_timeout(std::time::Duration::from_secs(600));
    renderer.initialize_with_device(device, queue)?;
    // PBR environment lighting for this frame: the session's own switch (IBL by
    // default) unless `--legacy-shading` asks for the pre-IBL single-light path.
    // This is what makes a metallic asset readable at all in a headless frame.
    let ibl_enabled = !invocation.legacy_shading && controller.ibl_enabled();
    renderer.set_ibl_enabled(ibl_enabled);
    // A budget or device-OOM refusal is reported with its own machine code and
    // exit 7 — never flattened into a generic GPU failure, never a blank PNG.
    if let Err(error) = renderer.upload(&delta) {
        return Err(staging_error(&renderer, error));
    }
    // Decoded images are a separate resident set: upload them after the batch
    // upload, and report any key the renderer could not resolve.
    let image_upload = match renderer.upload_images(&delta.images, &image_cache) {
        Ok(report) => report,
        Err(error) => return Err(staging_error(&renderer, error)),
    };
    // Mesh base-colour textures are a second residency set resolved from the
    // **same** decoded cache (ADR-0038 D10). Without this call a textured mesh
    // silently draws with its batch colour — the app path already uploaded them
    // (`cad-engine` session upload); the headless path must do the same or its
    // frame is not the document's frame. The keys come from the batches
    // themselves, so a scene with no textured batch is a no-op.
    let mesh_keys = cad_scene::mesh_texture_keys(&delta.added);
    let mesh_upload = match renderer.upload_mesh_textures(&mesh_keys, &image_cache) {
        Ok(report) => report,
        Err(error) => return Err(staging_error(&renderer, error)),
    };
    let target = RenderTarget::new(width, height);
    // The camera choice and the frame itself live in `camera::render_frame`:
    // the default is the unchanged 2D plan frame, while a requested view builds
    // the app's 3D camera state and renders the perspective frame.
    let (frame, camera_json) = camera::render_frame(
        &mut renderer,
        &target,
        requested_view,
        fitted.plan,
        fitted.bounds3,
        invocation.view_fov_degrees,
    )?;
    // Fallbacks are recorded **at draw time** (a batch whose key was not
    // resident), so they can only be read after the frame was encoded.
    let mesh_fallbacks = renderer.mesh_texture_fallbacks().to_vec();
    let readback = renderer.read_target_rgba().map_err(render_error);
    // The only PNG writer: it refuses to write anything unless a frame was
    // actually read back, so a refused render cannot leave an empty image.
    let (image, written) = write_png_for_frame(invocation.png.as_deref(), readback)?;
    let png_json = match &written {
        Some(png) => serde_json::json!({
            "path": png.path.display().to_string(),
            "bytes": png.bytes,
        }),
        None => serde_json::Value::Null,
    };
    // The renderer's own report, printed only after a successful frame (and the
    // optional PNG write): never a CLI estimate, and never printed on a refusal.
    print_gpu_memory_report(&renderer, invocation);

    let background = image.pixel(0, 0);
    let non_background = image.count_differing_from(background, 8);
    let coverage = non_background as f64 / (width as f64 * height as f64);

    let completeness = match &controller.last_import_report {
        Some(report) => completeness_json(&report.completeness),
        None => serde_json::json!({ "status": "unverified" }),
    };
    let scene_vertices: usize = delta.added.iter().map(|batch| batch.vertices.len()).sum();

    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::FixedViewportRender.as_str(),
        "adapter": {
            "backend": adapter.backend,
            "name": adapter.name,
            "device_type": adapter.device_type,
            "driver": adapter.driver,
            "driver_info": adapter.driver_info,
        },
        "width": width,
        "height": height,
        "png": png_json,
        "camera": camera_json,
        "pixels": {
            "non_background": non_background,
            "coverage": coverage,
            "distinct_colors": image.distinct_colors(),
        },
        "frame": {
            "draw_calls": frame.draw_calls,
            "vertices": frame.vertices,
            "triangles": frame.triangles,
            "opaque_batches": frame.opaque_batches,
            "transparent_batches": frame.transparent_batches,
            "invisible_batches": frame.invisible_batches,
        },
        "scene": {
            "batches": delta.added.len(),
            "vertices": scene_vertices,
        },
        "images": images_json(&image_report, &image_upload),
        "mesh_textures": mesh_textures_json(&mesh_keys, &mesh_upload, &mesh_fallbacks),
        "completeness": completeness,
        "note": "software/headless frame; not a compatibility or performance claim",
    }))
}

/// Native paper-space plot: render a named layout to a raster PNG or a vector
/// document (SVG/PDF).
///
/// The raster path is the honest paper-space chain: select the layout from the
/// database, read its plot settings (or an explicit documented default page),
/// plan the sheet onto a pixel canvas with `cad-representation::plot`, build the
/// paper-space representation (viewport transforms already applied), map that
/// paper geometry onto the canvas and render it through the same headless GPU
/// renderer `render` uses, then read back and encode PNG.
///
/// The vector path ([`PlotFormat::Svg`]/[`PlotFormat::Pdf`]) is entirely CPU: it
/// reuses the same layout selection, planner and paper-space representation,
/// then serialises the geometry as paths through
/// `cad-representation::plot_vector`. It creates **no** GPU device, so it works
/// on a machine with no adapter at all.
///
/// A drawing with no paper layout still exercises the planner and the writer
/// with a synthetic sheet, marked `"synthetic": true` in the report — never
/// claimed as a real layout plot.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_plot(invocation: &CliInvocation) -> Result<serde_json::Value, CliError> {
    if invocation.plot_format.is_vector() {
        return Ok(run_plot_vector(invocation)?);
    }
    run_plot_png(invocation)
}

/// CPU-only vector plot (SVG/PDF): no GPU device is created on this path.
#[cfg(not(target_arch = "wasm32"))]
fn run_plot_vector(invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    let started = std::time::Instant::now();
    let controller = load_document(invocation)?;
    let fonts = load_fonts(
        &invocation.fonts,
        &fonts_requested(&controller),
        drawing_needs_text(&controller),
        drawing_locale(&controller),
    )?;
    let context = representation_context(&controller, fonts.as_ref());
    let registry = cad_representation::ProviderRegistry::with_default_provider();

    let (database, layout, synthetic) = match select_plot_layout(&controller, invocation)? {
        Some((layout, id)) => (layout, id, false),
        None => (synthetic_plot_database(), cad_domain::LayoutId(0), true),
    };

    let record = database.plot_settings_for(layout);
    let target = plot_target(invocation, &record);
    let plan_started = std::time::Instant::now();
    let page = cad_representation::plan_plot_for_record(&record, target)?;
    let plan_ms = plan_started.elapsed().as_secs_f64() * 1000.0;

    let representation =
        cad_representation::build_paper_space(&registry, &database, layout, &context, &|entity| {
            !database.entity_hidden(entity.id)
        })?;
    let document = cad_representation::build_vector_document(&representation, &page)?;
    let path_count = document.paths.len();

    let bytes = match invocation.plot_format {
        PlotFormat::Svg => cad_representation::render_svg(&document)?,
        _ => cad_representation::render_pdf(&document)?,
    };
    let extension = invocation.plot_format.as_str();
    let output = invocation
        .png
        .clone()
        .unwrap_or_else(|| invocation.input.with_extension(format!("plot.{extension}")));
    write_atomic(&output, &bytes).map_err(|e| {
        CadError::InvalidInput(format!(
            "{} write failed: {}",
            extension.to_ascii_uppercase(),
            cad_diagnostics::redact_text(&e.to_string())
        ))
    })?;
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;

    // A base-colour texture cannot reach SVG/PDF (this path has no GPU device and
    // no image cache; the vector writers carry paths, not texels). Name the loss
    // rather than emit a document that looks like an untextured drawing.
    let mesh_texture_keys = representation_texture_keys(&representation);
    let mut diagnostics: Vec<serde_json::Value> = document
        .diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "code": d.code,
                "message": cad_diagnostics::redact_text(&d.message),
            })
        })
        .collect();
    if !mesh_texture_keys.is_empty() {
        diagnostics.push(serde_json::json!({
            // Same stable `vector.*` family as `cad-representation`'s own plot
            // diagnostics; this entry is the CLI-side one for a loss that is
            // known before the writers run.
            "code": "vector.mesh_texture_unsupported",
            "message": cad_diagnostics::redact_text(&format!(
                "mesh base-colour texture(s) {} are not written to {}: the vector plot \
                 carries geometry only; use --plot-format png for a textured sheet",
                mesh_texture_keys.join(","),
                invocation.plot_format.as_str().to_ascii_uppercase(),
            )),
        }));
    }

    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Plot.as_str(),
        "format": invocation.plot_format.as_str(),
        "layout": {
            "id": layout.0,
            "name": database.layout(layout).map(|l| l.name.clone()).unwrap_or_else(|| "synthetic".into()),
            "synthetic": synthetic,
        },
        "width_mm": document.width_mm,
        "height_mm": document.height_mm,
        "paths": path_count,
        "paper": paper_json(&record, &page),
        "output": output.display().to_string(),
        "bytes": bytes.len(),
        "completeness": completeness_json(&document.completeness),
        "diagnostics": diagnostics,
        "timings": { "plan_ms": plan_ms, "total_ms": total_ms },
        "status": "ok",
        "note": "CPU-only vector plot: path-level geometry, no font embedding and no plot styles (see docs/plot.md)",
    }))
}

/// The `plot` canvas target: an explicit DPI, else the `--width`/`--height`
/// canvas. Shared by the raster and vector paths so they cannot drift.
#[cfg(not(target_arch = "wasm32"))]
fn plot_target(
    invocation: &CliInvocation,
    _record: &cad_db::PlotSettingsRecord,
) -> cad_representation::PlotTarget {
    match invocation.plot_dpi {
        Some(dpi) => cad_representation::PlotTarget::Dpi(dpi),
        None => cad_representation::PlotTarget::Pixels {
            width: invocation.render_width,
            height: invocation.render_height,
        },
    }
}

/// The stable `paper` object of the plot result JSON.
#[cfg(not(target_arch = "wasm32"))]
fn paper_json(
    record: &cad_db::PlotSettingsRecord,
    page: &cad_representation::PlotPage,
) -> serde_json::Value {
    let (provenance, default_reason) = match &record.provenance {
        cad_db::PlotProvenance::Imported => ("imported", serde_json::Value::Null),
        cad_db::PlotProvenance::Authored => ("authored", serde_json::Value::Null),
        cad_db::PlotProvenance::DefaultPage { reason } => {
            ("default_page", serde_json::Value::String(reason.clone()))
        }
    };
    serde_json::json!({
        "size_name": record.paper_size_name,
        "width": record.paper_width,
        "height": record.paper_height,
        "units": format!("{:?}", record.paper_units).to_ascii_lowercase(),
        "rotation_degrees": page.rotation_degrees,
        "printable_mm": [page.printable_mm.0, page.printable_mm.1],
        "scale": {
            "numerator": record.scale_numerator,
            "denominator": record.scale_denominator,
            "factor": cad_representation::PlotScale {
                numerator: record.scale_numerator,
                denominator: record.scale_denominator,
            }
            .factor(),
        },
        "provenance": provenance,
        "default_reason": default_reason,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_plot_png(invocation: &CliInvocation) -> Result<serde_json::Value, CliError> {
    use cad_render_wgpu::headless::{create_headless_gpu_with, HeadlessGpu};
    use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};

    let started = std::time::Instant::now();
    let controller = load_document(invocation)?;
    let fonts = load_fonts(
        &invocation.fonts,
        &fonts_requested(&controller),
        drawing_needs_text(&controller),
        drawing_locale(&controller),
    )?;
    let (image_cache, image_report) = load_images(invocation, &controller)?;
    let mut context = representation_context(&controller, fonts.as_ref());
    // File images and document-owned mesh textures both attach the cache: a plot
    // that carries only textured meshes must still decode them (ADR-0038 D10).
    if !image_report.requested.is_empty() {
        context = context.with_images(image_cache.clone());
    }
    let registry = cad_representation::ProviderRegistry::with_default_provider();

    // Real layouts when present, otherwise a synthetic sheet so the planner and
    // encoder are still exercised. The report marks which path was taken.
    let (database, layout, synthetic) = match select_plot_layout(&controller, invocation)? {
        Some((layout, id)) => (layout, id, false),
        None => (synthetic_plot_database(), cad_domain::LayoutId(0), true),
    };

    let record = database.plot_settings_for(layout);
    let target = plot_target(invocation, &record);
    let plan_started = std::time::Instant::now();
    let page = cad_representation::plan_plot_for_record(&record, target)?;
    let plan_ms = plan_started.elapsed().as_secs_f64() * 1000.0;

    // Paper-space representation, already mapped through every viewport. The
    // source-baked `invisible` flag is honoured exactly like the GUI scene; layer
    // visibility stays out of the plot decision, as before.
    let representation =
        cad_representation::build_paper_space(&registry, &database, layout, &context, &|entity| {
            !database.entity_hidden(entity.id)
        })?;

    // Map paper millimetres onto the renderer's y-up pixel space: the planner's
    // affine is y-down (image convention), so flip y about the canvas height.
    let paper_to_render = paper_to_render_transform(&page);
    let mut plan = cad_scene::SceneCache::new(Default::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    };
    let mut batches: u64 = 0;
    let mut vertices: u64 = 0;
    for fragment in &representation.fragments {
        let transformed = cad_representation::DisplayFragment {
            source: fragment.source.clone(),
            geometry_source: fragment.geometry_source.clone(),
            precision: fragment.precision.clone(),
            alpha: fragment.alpha,
            color: fragment.color,
            color_unresolved: fragment.color_unresolved,
            lineweight: fragment.lineweight,
            lineweight_unresolved: fragment.lineweight_unresolved,
            linetype: fragment.linetype.clone(),
            linetype_scale: fragment.linetype_scale,
            linetype_unresolved: fragment.linetype_unresolved,
            draw_order: fragment.draw_order,
            primitive: fragment.primitive.transformed(&paper_to_render),
        };
        let only = cad_representation::DisplayRepresentation {
            fragments: vec![transformed],
            completeness: representation.completeness.clone(),
            diagnostics: representation.diagnostics.clone(),
        };
        let built = plan.build(&only, context.stamp.clone())?;
        batches = batches.saturating_add(built.added.len() as u64);
        vertices = vertices.saturating_add(
            built
                .added
                .iter()
                .map(|batch| batch.vertices.len() as u64)
                .sum::<u64>(),
        );
        enforce_scene_budget(batches, vertices, invocation)?;
        delta.added.extend(built.added);
        delta.images.extend(built.images);
        delta.masks.extend(built.masks);
    }

    let width = page.width;
    let height = page.height;
    let camera = Camera2d {
        center: Point3 {
            x: width as f64 / 2.0,
            y: height as f64 / 2.0,
            z: 0.0,
        },
        world_per_px: 1.0,
        z_plane: 0.0,
    };

    let gpu_started = std::time::Instant::now();
    let HeadlessGpu {
        device,
        queue,
        adapter,
    } = create_headless_gpu_with(BackendPreference::Auto, render_gpu(invocation))?;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // Same policy as `render`: flag > `YACR_GPU_BUDGET_MB` > default estimate.
    apply_resolved_gpu_budget(&mut renderer, invocation);
    renderer.set_poll_timeout(std::time::Duration::from_secs(600));
    renderer.initialize_with_device(device, queue)?;
    if let Err(error) = renderer.upload(&delta) {
        return Err(staging_error(&renderer, error));
    }
    let image_upload = match renderer.upload_images(&delta.images, &image_cache) {
        Ok(report) => report,
        Err(error) => return Err(staging_error(&renderer, error)),
    };
    // Same textured-mesh residency as `render`: a paper-space viewport can show a
    // textured mesh, and the plot frame must sample it instead of the batch
    // colour. Empty for an untextured sheet, so nothing extra is uploaded.
    let mesh_keys = cad_scene::mesh_texture_keys(&delta.added);
    let mesh_upload = match renderer.upload_mesh_textures(&mesh_keys, &image_cache) {
        Ok(report) => report,
        Err(error) => return Err(staging_error(&renderer, error)),
    };
    let render_target = RenderTarget::new(width, height);
    let frame = renderer
        .render(camera, &render_target)
        .map_err(render_error)?;
    // Fallbacks are recorded at draw time, so they are read after the frame.
    let mesh_fallbacks = renderer.mesh_texture_fallbacks().to_vec();
    let readback = renderer.read_target_rgba().map_err(render_error);
    // The plot always names an output file (defaulting from the input), so the
    // frame-gated writer is given `Some`; a refused frame still writes nothing.
    let output = invocation
        .png
        .clone()
        .unwrap_or_else(|| invocation.input.with_extension("plot.png"));
    let (image, png) = write_required_png(&output, readback)?;
    let png_bytes = png.bytes;
    // The renderer's own report, published only after a successful frame.
    print_gpu_memory_report(&renderer, invocation);

    let gpu_ms = gpu_started.elapsed().as_secs_f64() * 1000.0;

    let background = image.pixel(0, 0);
    let non_background = image.count_differing_from(background, 8);
    let coverage = non_background as f64 / (width as f64 * height as f64);

    let total_ms = started.elapsed().as_secs_f64() * 1000.0;

    let completeness = match &controller.last_import_report {
        Some(report) => completeness_json(&report.completeness),
        None => serde_json::json!({ "status": "unverified" }),
    };

    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Plot.as_str(),
        "format": invocation.plot_format.as_str(),
        "layout": {
            "id": layout.0,
            "name": database.layout(layout).map(|l| l.name.clone()).unwrap_or_else(|| "synthetic".into()),
            "synthetic": synthetic,
        },
        "width": width,
        "height": height,
        "dpi": page.dpi,
        "paper": paper_json(&record, &page),
        "output": output.display().to_string(),
        "bytes": png_bytes,
        "pixels": {
            "non_background": non_background,
            "coverage": coverage,
            "distinct_colors": image.distinct_colors(),
        },
        "frame": {
            "draw_calls": frame.draw_calls,
            "vertices": frame.vertices,
            "triangles": frame.triangles,
            "opaque_batches": frame.opaque_batches,
            "transparent_batches": frame.transparent_batches,
            "invisible_batches": frame.invisible_batches,
        },
        "images": images_json(&image_report, &image_upload),
        "mesh_textures": mesh_textures_json(&mesh_keys, &mesh_upload, &mesh_fallbacks),
        "timings": { "plan_ms": plan_ms, "gpu_ms": gpu_ms, "total_ms": total_ms },
        "adapter": {
            "backend": adapter.backend,
            "name": adapter.name,
            "device_type": adapter.device_type,
        },
        "completeness": completeness,
        "status": "ok",
        "note": "raster paper-space plot; use --plot-format svg|pdf for the CPU vector path; no plot styles (see docs/plot.md)",
    }))
}

/// Resolve the layout to plot, or `None` when the drawing has no paper layout.
#[cfg(not(target_arch = "wasm32"))]
fn select_plot_layout(
    controller: &HostController,
    invocation: &CliInvocation,
) -> CadResult<Option<(cad_db::DrawingDatabase, cad_domain::LayoutId)>> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let database = document.drawing.as_ref().clone();
    // Plot targets paper-space layouts only. The importer synthesises a
    // `LayoutId(0)` "Model" layout when a drawing has no paper space; that is
    // model space, not a sheet, so it must not be selected as a plot layout.
    let descriptors: Vec<_> = cad_representation::enumerate_layouts(&database)
        .into_iter()
        .filter(|d| !(d.id == cad_domain::LayoutId(0) && d.name == "Model"))
        .collect();
    if descriptors.is_empty() {
        return Ok(None);
    }
    let id = match &invocation.layout {
        Some(name) => descriptors
            .iter()
            .find(|d| d.name == *name || d.name.eq_ignore_ascii_case(name))
            .map(|d| d.id)
            .ok_or_else(|| {
                let available: Vec<&str> = descriptors.iter().map(|d| d.name.as_str()).collect();
                CadError::InvalidInput(format!(
                    "layout '{name}' not found; available: {}",
                    available.join(", ")
                ))
            })?,
        // Prefer the populated sheet over an empty first paper layout.
        // AutoCAD/QCAD files commonly contain an empty paper layout first; its
        // populated sibling is the real sheet, and plotting the empty one would
        // show a blank (or model-space fallback) frame.
        None => match descriptors.iter().max_by_key(|d| d.viewport_count) {
            Some(richest) if richest.viewport_count > 0 => richest.id,
            _ => descriptors[0].id,
        },
    };
    Ok(Some((database, id)))
}

/// A minimal in-memory sheet used when a drawing has no paper layout.
///
/// It is deliberately marked synthetic in the report: it exercises the planner
/// and the encoder, but it is not evidence about the imported drawing.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn synthetic_plot_database() -> cad_db::DrawingDatabase {
    use cad_db::{DbEntity, DbObject, Layer, Layout};
    use cad_domain::{EntityId, LayerId, ObjectId, Revision, SpaceId};

    let mut builder = cad_db::DrawingDatabaseBuilder::new(cad_domain::DatabaseId(0));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: cad_domain::LayoutId(0),
            name: "Synthetic".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    // A rectangle frame plus a diagonal, in paper millimetres inside A4.
    let corners = [
        (20.0, 20.0, 190.0, 20.0),
        (190.0, 20.0, 190.0, 277.0),
        (190.0, 277.0, 20.0, 277.0),
        (20.0, 277.0, 20.0, 20.0),
        (20.0, 20.0, 190.0, 277.0),
    ];
    for (index, (x1, y1, x2, y2)) in corners.into_iter().enumerate() {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(index as u128 + 1),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(index as u128 + 1),
                layer: LayerId(0),
                space: SpaceId::Paper(cad_domain::LayoutId(0)),
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: x1,
                        y: y1,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: x2,
                        y: y2,
                        z: 0.0,
                    },
                },
                draw_order: index as i64,
            })
            .unwrap();
    }
    builder.finish().expect("synthetic plot database is valid")
}

/// A PNG that was actually written for a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrittenPng {
    /// The path the bytes were written to (the caller's `--png`, or the plot's
    /// default file).
    pub path: PathBuf,
    /// Size of the encoded PNG in bytes.
    pub bytes: usize,
}

/// The **only** PNG writer on the GPU paths: a frame must exist first.
///
/// `frame` is the read-back result, so this function cannot be reached with a
/// refused or failed render: an `Err` is returned unchanged and **nothing** is
/// touched on disk. That is the contract behind "never a blank PNG written as if
/// it worked": a GPU-memory refusal (or a device loss, or a rejected frame)
/// aborts before a byte is written, leaving no file and no partial file.
///
/// `write_atomic` keeps the file-level half of the same promise (temp file +
/// rename), so even a disk failure cannot leave a truncated PNG.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_png_for_frame(
    path: Option<&Path>,
    frame: Result<cad_render_wgpu::headless::RgbaImage, CliError>,
) -> Result<(cad_render_wgpu::headless::RgbaImage, Option<WrittenPng>), CliError> {
    use cad_render_wgpu::headless::encode_png;

    let image = frame?;
    let Some(path) = path else {
        return Ok((image, None));
    };
    let bytes = encode_png(&image)
        .map_err(|e| CliError::new(error_code::INVARIANT, format!("PNG encode failed: {e}")))?;
    write_atomic(path, &bytes).map_err(|e| {
        CliError::new(
            error_code::INVALID_INPUT,
            format!(
                "PNG write failed: {}",
                cad_diagnostics::redact_text(&e.to_string())
            ),
        )
        .with_context(serde_json::json!({ "path": path.display().to_string() }))
    })?;
    Ok((
        image,
        Some(WrittenPng {
            path: path.to_path_buf(),
            bytes: bytes.len(),
        }),
    ))
}

/// [`write_png_for_frame`] for a path that must produce a file (the plot's
/// output, which always has a name), failing loudly instead of reporting zero
/// bytes if that ever stops being true.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_required_png(
    path: &Path,
    frame: Result<cad_render_wgpu::headless::RgbaImage, CliError>,
) -> Result<(cad_render_wgpu::headless::RgbaImage, WrittenPng), CliError> {
    let (image, written) = write_png_for_frame(Some(path), frame)?;
    match written {
        Some(written) => Ok((image, written)),
        None => Err(CliError::new(
            error_code::INVARIANT,
            "internal: an output path was given but no PNG was written",
        )),
    }
}

/// Paper millimetres → renderer pixel space (y-up), as a [`Transform3`].
#[cfg(not(target_arch = "wasm32"))]
fn paper_to_render_transform(page: &cad_representation::PlotPage) -> Transform3 {
    let m = page.paper_to_pixel;
    let height = page.height as f64;
    let mut out = Transform3::identity().matrix;
    out[0][0] = m[0];
    out[0][1] = m[2];
    out[0][3] = m[4];
    out[1][0] = -m[1];
    out[1][1] = -m[3];
    out[1][3] = height - m[5];
    Transform3 { matrix: out }
}

/// The stable `images` object of the `render`/`plot` result JSON.
///
/// `unresolved` merges the loader's logical misses with the uploader's
/// sanitized misses, so a key that never reached the GPU is visible exactly
/// once and never read as a silent drop.
#[cfg(not(target_arch = "wasm32"))]
fn images_json(
    report: &cad_platform::images::ImageLoadReport,
    upload: &cad_render_wgpu::ImageUploadReport,
) -> serde_json::Value {
    let mut unresolved: Vec<String> = report.unresolved.clone();
    for key in &upload.unresolved {
        let key = key.as_str().to_string();
        if !unresolved.iter().any(|existing| existing == &key) {
            unresolved.push(key);
        }
    }
    serde_json::json!({
        "requested": report.requested,
        "loaded": report.loaded,
        "unresolved": unresolved,
        "failed": report.failed,
        "uploaded": upload.uploaded,
        "bytes": report.bytes,
    })
}

/// The stable `mesh_textures` object of the `render`/raster-`plot` result JSON.
///
/// A mesh base-colour texture is a fact about the **frame**, not an internal
/// detail: `requested` names every key the resident batches reference,
/// `uploaded` counts the ones that became device textures, and
/// `unresolved`/`fallbacks` name every key that did not (an unresolved key is
/// missing from the cache; a fallback key was wanted by a drawn batch but was
/// not resident, so that batch was drawn with its batch colour). The renderer
/// never binds a placeholder, so this object is the only place that degradation
/// is visible. `requested: []` is therefore the one shape that means "this
/// drawing has no mesh textures", and it costs no GPU work.
#[cfg(not(target_arch = "wasm32"))]
fn mesh_textures_json(
    requested: &[cad_resources::ResourceKey],
    upload: &cad_render_wgpu::ImageUploadReport,
    fallbacks: &[cad_resources::ResourceKey],
) -> serde_json::Value {
    let keys = |items: &[cad_resources::ResourceKey]| {
        items
            .iter()
            .map(|key| key.as_str().to_string())
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "requested": keys(requested),
        "uploaded": upload.uploaded,
        "unresolved": keys(&upload.unresolved),
        "fallbacks": keys(fallbacks),
        "bytes": upload.bytes,
    })
}

/// Every base-colour texture key a paper-space representation names.
///
/// The CPU vector plot has no decoded-image cache and no GPU texture path, so a
/// textured mesh plot as paths: this is how the vector path notices the loss
/// instead of writing a document that is silently untextured.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn representation_texture_keys(
    representation: &cad_representation::DisplayRepresentation,
) -> Vec<String> {
    use cad_representation::DisplayPrimitive;

    let mut keys = std::collections::BTreeSet::new();
    for fragment in &representation.fragments {
        let mesh = match &fragment.primitive {
            DisplayPrimitive::Mesh(mesh)
            | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh)
            | DisplayPrimitive::MeshWithStoredEdges { mesh, .. } => Some(mesh),
            _ => None,
        };
        if let Some(texture) = mesh.and_then(|mesh| mesh.texture.as_deref()) {
            keys.insert(texture.to_owned());
        }
    }
    keys.into_iter().collect()
}
