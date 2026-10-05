//! ops module.

use super::*;

pub(crate) fn run_scan(controller: &HostController) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let bounds = document.drawing.bounds().map(|(min, max)| {
        serde_json::json!({ "min": [min.x, min.y, min.z], "max": [max.x, max.y, max.z] })
    });
    let (completeness, diagnostics) = match &controller.last_import_report {
        Some(report) => (
            completeness_json(&report.completeness),
            report
                .diagnostics
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "code": d.code,
                        "message": cad_diagnostics::redact_text(&d.message),
                    })
                })
                .collect::<Vec<_>>(),
        ),
        None => (serde_json::json!({ "status": "unverified" }), Vec::new()),
    };
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Scan.as_str(),
        "entities": document.drawing.entity_count(),
        "model_entities": document.drawing.model_space().len(),
        "block_definitions": document.drawing.blocks().count(),
        "layers": document.drawing.layers().count(),
        "bounds": bounds,
        "units": units_name(&document.units),
        "completeness": completeness,
        "diagnostics": diagnostics,
    }))
}

pub(crate) fn run_proxy_report(controller: &HostController) -> CadResult<serde_json::Value> {
    let report = controller
        .last_import_report
        .as_ref()
        .ok_or_else(|| CadError::InvalidInput("no import report; open a DWG first".into()))?;
    // Every proxy/unknown diagnostic is surfaced, not just a subset: a proxy
    // without a cache is a completeness issue, not silent success (audit B30).
    let proxy_diagnostics = import_completeness_issues(report);
    let types: Vec<_> = report
        .capabilities
        .iter()
        .map(|capability| {
            serde_json::json!({
                "type": capability.type_key,
                "read": support_name(capability.read),
                "semantic": support_name(capability.semantic),
                "render": support_name(capability.render),
                "pick": support_name(capability.pick),
                "measure": support_name(capability.measure),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::ProxyReport.as_str(),
        "completeness": completeness_json(&report.completeness),
        "entity_types": types,
        "proxy_diagnostics": proxy_diagnostics,
        "note": "entity types are reported per capability; absence of a type is not a compatibility claim",
    }))
}

pub(crate) fn run_measure(
    controller: &mut HostController,
    invocation: &CliInvocation,
) -> CadResult<serde_json::Value> {
    if invocation.points.len() < 2 {
        return Err(CadError::InvalidInput(
            "measure needs two (distance), three (angle) or more (length) points".into(),
        ));
    }
    let command = Command {
        schema_version: 1,
        id: CommandId::Measure,
        document: controller.document_id,
        viewport: controller.viewport_id,
        payload: CommandPayload::Points(invocation.points.clone()),
    };
    let outcome = controller.execute(command)?;
    let units = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .map(|d| units_name(&d.units))
        .unwrap_or_else(|| "unknown".to_string());
    let measurement = outcome.measurement.as_ref().map(measurement_json);
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::Measure.as_str(),
        "input_points": invocation.points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
        "measurement": measurement,
        "results": outcome.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "message": d.message,
        })).collect::<Vec<_>>(),
        "units": units,
    }))
}

/// The representation context shared by `build-representation` and `render`.
///
/// Both must build from the same provider registry, document id, tolerance
/// policy and task stamp; only this constructor is allowed to define that, so
/// the two paths cannot silently drift apart.
pub(crate) fn representation_context(
    controller: &HostController,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> cad_representation::RepresentationContext {
    let mut context = cad_representation::RepresentationContext::new(
        controller.document_id,
        TolerancePolicy::default(),
        TaskStamp::new(controller.document_id, controller.session.generation),
    );
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts.clone());
    }
    context
}

pub(crate) fn run_build_representation(
    controller: &HostController,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> CadResult<serde_json::Value> {
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = representation_context(controller, fonts);
    let (mut lines, mut meshes, mut texts, mut instances, mut images) = (0usize, 0, 0, 0, 0);
    let mut vertices = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for entity in document.drawing.model_space() {
        match registry.build_expanded(&document.drawing, entity, &context) {
            Ok(representation) => {
                for fragment in &representation.fragments {
                    match &fragment.primitive {
                        cad_representation::DisplayPrimitive::Lines(points)
                        | cad_representation::DisplayPrimitive::LineSegments(points) => {
                            lines += 1;
                            vertices += points.len();
                        }
                        cad_representation::DisplayPrimitive::Mesh(mesh) => {
                            meshes += 1;
                            vertices += mesh.vertices.len();
                        }
                        cad_representation::DisplayPrimitive::Text { .. } => texts += 1,
                        cad_representation::DisplayPrimitive::Instance { .. } => instances += 1,
                        cad_representation::DisplayPrimitive::Image { .. } => images += 1,
                    }
                }
            }
            Err(error) => failures.push(serde_json::json!({
                "entity": entity.id.0.to_string(),
                "error": error.to_string(),
            })),
        }
    }
    let primitives = lines + meshes + texts + instances + images;
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "operation": CliOperation::BuildRepresentation.as_str(),
        "primitives": primitives,
        "vertices": vertices,
        "kind_counts": {
            "lines": lines,
            "meshes": meshes,
            "texts": texts,
            "instances": instances,
            "images": images,
        },
        "failures": failures,
    }))
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
pub(crate) fn run_render(invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu};
    use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};

    let controller = load_document(invocation)?;
    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let fonts = load_fonts(&invocation.fonts)?;
    let context = representation_context(&controller, fonts.as_ref());

    // One delta over every model-space entity. `SceneCache::build` skips
    // Text/Instance/Image (documented) and returns line/mesh batches only.
    let mut cache = cad_scene::SceneCache::new(Default::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
    for entity in document.drawing.model_space() {
        let representation = registry.build_expanded(&document.drawing, entity, &context)?;
        let built = cache.build(&representation, context.stamp.clone())?;
        delta.added.extend(built.added);
    }

    // Fit to what is actually drawn, not `drawing.bounds()`: the database
    // bounds include material the renderer does not draw (text, unplaced or
    // block-definition geometry), which leaves the framed image off-centre and
    // small. The scene batches already carry world-space points
    // (`local_origin + vertex`), so derive the fit from those.
    let mut fit: Option<(f64, f64, f64, f64)> = None; // min_x, min_y, max_x, max_y
    for batch in &delta.added {
        let ox = batch.local_origin.x;
        let oy = batch.local_origin.y;
        for vertex in &batch.vertices {
            let x = ox + vertex[0] as f64;
            let y = oy + vertex[1] as f64;
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            fit = Some(match fit {
                None => (x, y, x, y),
                Some((min_x, min_y, max_x, max_y)) => {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }
            });
        }
    }
    let (min_x, min_y, max_x, max_y) =
        fit.ok_or_else(|| CadError::InvalidInput("no drawable geometry to render".into()))?;

    let width = invocation.render_width;
    let height = invocation.render_height;
    let span_x = (max_x - min_x).abs();
    let span_y = (max_y - min_y).abs();
    let world_per_px = (span_x / (width as f64 * 0.9))
        .max(span_y / (height as f64 * 0.9))
        .max(1e-9);
    let camera = Camera2d {
        center: Point3 {
            x: (min_x + max_x) / 2.0,
            y: (min_y + max_y) / 2.0,
            z: 0.0,
        },
        world_per_px,
        z_plane: 0.0,
    };

    let HeadlessGpu {
        device,
        queue,
        adapter,
    } = create_headless_gpu(BackendPreference::Auto)?;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    // A large drawing is tens of thousands of small draw calls; a CPU software
    // adapter can legitimately exceed the interactive 1 s submission bound.
    // Headless evidence waits longer instead of misreporting a device loss.
    renderer.set_poll_timeout(std::time::Duration::from_secs(600));
    renderer.initialize_with_device(device, queue)?;
    renderer.upload(&delta)?;
    let target = RenderTarget::new(width, height);
    let frame = renderer
        .render(camera, &target)
        .map_err(|error| CadError::GpuFailure(error.message().to_string()))?;
    let image = renderer
        .read_target_rgba()
        .map_err(|error| CadError::GpuFailure(error.message().to_string()))?;

    let background = image.pixel(0, 0);
    let non_background = image.count_differing_from(background, 8);
    let coverage = non_background as f64 / (width as f64 * height as f64);

    // Only after a successful frame: encode and atomically write, so a failure
    // never leaves a partial PNG.
    let png_json = match &invocation.png {
        Some(path) => {
            let bytes = encode_png(&image)
                .map_err(|e| CadError::Invariant(format!("PNG encode failed: {e}")))?;
            write_atomic(path, &bytes).map_err(|e| {
                CadError::InvalidInput(format!(
                    "PNG write failed: {}",
                    cad_diagnostics::redact_text(&e.to_string())
                ))
            })?;
            serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
            })
        }
        None => serde_json::Value::Null,
    };

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
pub(crate) fn run_plot(invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    if invocation.plot_format.is_vector() {
        return run_plot_vector(invocation);
    }
    run_plot_png(invocation)
}

/// CPU-only vector plot (SVG/PDF): no GPU device is created on this path.
#[cfg(not(target_arch = "wasm32"))]
fn run_plot_vector(invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    let started = std::time::Instant::now();
    let controller = load_document(invocation)?;
    let fonts = load_fonts(&invocation.fonts)?;
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
        cad_representation::build_paper_space(&registry, &database, layout, &context, &|_| true)?;
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
        "diagnostics": document.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "message": cad_diagnostics::redact_text(&d.message),
        })).collect::<Vec<_>>(),
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
pub(crate) fn run_plot_png(invocation: &CliInvocation) -> CadResult<serde_json::Value> {
    use cad_render_wgpu::headless::{create_headless_gpu, encode_png, HeadlessGpu};
    use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};

    let started = std::time::Instant::now();
    let controller = load_document(invocation)?;
    let fonts = load_fonts(&invocation.fonts)?;
    let context = representation_context(&controller, fonts.as_ref());
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

    // Paper-space representation, already mapped through every viewport.
    let representation =
        cad_representation::build_paper_space(&registry, &database, layout, &context, &|_| true)?;

    // Map paper millimetres onto the renderer's y-up pixel space: the planner's
    // affine is y-down (image convention), so flip y about the canvas height.
    let paper_to_render = paper_to_render_transform(&page);
    let mut plan = cad_scene::SceneCache::new(Default::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
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
            primitive: fragment.primitive.transformed(&paper_to_render),
        };
        let only = cad_representation::DisplayRepresentation {
            fragments: vec![transformed],
            completeness: representation.completeness.clone(),
            diagnostics: representation.diagnostics.clone(),
        };
        delta
            .added
            .extend(plan.build(&only, context.stamp.clone())?.added);
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
    } = create_headless_gpu(BackendPreference::Auto)?;
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_poll_timeout(std::time::Duration::from_secs(600));
    renderer.initialize_with_device(device, queue)?;
    renderer.upload(&delta)?;
    let render_target = RenderTarget::new(width, height);
    let frame = renderer
        .render(camera, &render_target)
        .map_err(|error| CadError::GpuFailure(error.message().to_string()))?;
    let image = renderer
        .read_target_rgba()
        .map_err(|error| CadError::GpuFailure(error.message().to_string()))?;
    let gpu_ms = gpu_started.elapsed().as_secs_f64() * 1000.0;

    let background = image.pixel(0, 0);
    let non_background = image.count_differing_from(background, 8);
    let coverage = non_background as f64 / (width as f64 * height as f64);

    let png_bytes =
        encode_png(&image).map_err(|e| CadError::Invariant(format!("PNG encode failed: {e}")))?;
    let output = invocation
        .png
        .clone()
        .unwrap_or_else(|| invocation.input.with_extension("plot.png"));
    write_atomic(&output, &png_bytes).map_err(|e| {
        CadError::InvalidInput(format!(
            "PNG write failed: {}",
            cad_diagnostics::redact_text(&e.to_string())
        ))
    })?;
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
        "bytes": png_bytes.len(),
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

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_benchmark(
    controller: &HostController,
    invocation: &CliInvocation,
    fonts: Option<&Arc<cad_representation::FontEngine>>,
) -> CadResult<serde_json::Value> {
    use cad_diagnostics::{
        BenchmarkBudgets, BenchmarkReport, MeasuredContext, MeasuredMemory, MeasuredTimings,
    };

    // `file_bytes` is the real on-disk size of the opened sample.
    let file_bytes = std::fs::metadata(&invocation.input).ok().map(|m| m.len());

    // Parse time was measured by the importer on this exact open; it is `None`
    // if this path never ran the importer (for example a caller reusing a
    // pre-loaded document), and we report `null` rather than fake a number.
    let parse_ms = controller
        .last_import_report
        .as_ref()
        .and_then(|report| report.parse_ms);

    let document = controller
        .application
        .workspace
        .documents
        .get(&controller.document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = representation_context(controller, fonts);

    // Build the display representation and batch it, timing the geometry build
    // and measuring CPU cache bytes exactly (SceneCache counts every chunk).
    let mut cache = cad_scene::SceneCache::new(cad_scene::SceneBudget::default());
    let mut delta = cad_scene::SceneDelta {
        stamp: context.stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
    let build_start = std::time::Instant::now();
    let mut primitives = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for entity in document.drawing.model_space() {
        match registry.build_expanded(&document.drawing, entity, &context) {
            Ok(representation) => {
                primitives += representation.fragments.len();
                let built = cache.build(&representation, context.stamp.clone())?;
                delta.added.extend(built.added);
            }
            Err(error) => failures.push(serde_json::json!({
                "entity": entity.id.0.to_string(),
                "error": error.to_string(),
            })),
        }
    }
    let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;

    // GPU estimate is the exact packed buffer size the renderer would upload for
    // these batches, summed from the scene itself (not a heuristic). Computed
    // before publishing moves the batches into the cache.
    let gpu_estimated_bytes: u64 = delta
        .added
        .iter()
        .map(|batch| batch.upload_size_bytes() as u64)
        .sum();
    let vertices: usize = delta.added.iter().map(|b| b.vertices.len()).sum();
    let triangles: usize = delta
        .added
        .iter()
        .filter(|b| b.topology == cad_scene::RenderTopology::Mesh)
        .map(|b| b.triangle_count())
        .sum();
    let batches = delta.added.len();

    // Publish so the CPU cache accounts the bytes exactly as the live cache
    // would; `total_cpu_bytes` is the real accounted geometry footprint.
    cache.publish(delta, &context.stamp)?;
    let cpu_geometry_bytes = cache.total_cpu_bytes() as u64;

    // This native CLI path does not own a GPU device, so upload and
    // first-usable frame times are not measurable here. They stay `None`; the
    // `benchmark-gpu` (render) path owns those phases. Never a fabricated 0.
    let sample_hash = controller
        .last_import_report
        .as_ref()
        .and_then(sample_hash_of);
    let measured_context = MeasuredContext {
        sample_hash,
        // No GPU device is selected on this geometry path.
        device: None,
        // The CLI is not a browser.
        browser: None,
        release_build: !cfg!(debug_assertions),
        // The controlling session viewport is real and known.
        viewport: Some(controller.viewport_id),
        quality_configuration: Some("scene-budget-default".to_string()),
    };
    let report = BenchmarkReport {
        // Raw sample bytes are not retained here, so the sample identity is the
        // importer's content hash carried on the report, `None` when this path
        // did not import. It is never a path or a file name.
        sample_hash,
        release_build: !cfg!(debug_assertions),
        timings: MeasuredTimings {
            parse_ms,
            build_ms: Some(build_ms),
            upload_ms: None,
            first_usable_ms: None,
            complete_ms: Some(build_ms),
        },
        memory: MeasuredMemory {
            file_bytes,
            domain_bytes: None, // no byte-exact database estimate exists
            cpu_geometry_bytes: Some(cpu_geometry_bytes),
            gpu_estimated_bytes: Some(gpu_estimated_bytes),
            atlas_bytes: None,      // no atlas is built on this path
            attachment_bytes: None, // no render attachments on this path
        },
        budgets: BenchmarkBudgets {
            cpu_bytes: Some(cad_scene::SceneBudget::default().cpu_bytes as u64),
            upload_bytes_per_frame: Some(
                cad_scene::SceneBudget::default().upload_bytes_per_frame as u64,
            ),
            queued_tasks: Some(cad_scene::SceneBudget::default().queued_tasks as u64),
            max_vertices_per_frame: Some(
                cad_scene::SceneBudget::default().max_vertices_per_frame as u64,
            ),
            max_triangles_per_frame: Some(
                cad_scene::SceneBudget::default().max_triangles_per_frame as u64,
            ),
        },
        context: measured_context,
        over_budget: Vec::new(),
    };

    let mut value = report.to_json();
    // Keep the operation/schema envelope the other CLI documents use, and the
    // scene counts as measured facts (not a performance grade). `batches` is
    // captured before the cache took ownership of the delta.
    value["schema_version"] = serde_json::json!(CLI_SCHEMA_VERSION);
    value["operation"] = serde_json::json!(CliOperation::Benchmark.as_str());
    value["scene"] = serde_json::json!({
        "primitives": primitives,
        "batches": batches,
        "vertices": vertices,
        "triangles": triangles,
    });
    value["failures"] = serde_json::json!(failures);
    Ok(value)
}

/// Recover the content hash from an import report's identity, if it carries one.
///
/// `DocumentIdentity::Temporary` carries no content hash, so this returns `None`
/// and the benchmark leaves the hash absent rather than inventing one.
#[cfg(not(target_arch = "wasm32"))]
fn sample_hash_of(report: &cad_import_acadrust::ImportReport) -> Option<[u8; 32]> {
    match &report.identity {
        DocumentIdentity::Sha256(hash) => Some(*hash),
        _ => None,
    }
}

/// Load the `--font` entries into a shaping engine, if any were given.
pub(crate) fn load_fonts(
    entries: &[(String, PathBuf)],
) -> CadResult<Option<Arc<cad_representation::FontEngine>>> {
    if entries.is_empty() {
        return Ok(None);
    }
    let mut engine = cad_representation::FontEngine::new();
    let mut keys = Vec::new();
    for (key, path) in entries {
        let bytes = std::fs::read(path)
            .map_err(|e| CadError::InvalidInput(format!("font read failed: {e}")))?;
        engine.register(key, Arc::from(bytes.into_boxed_slice()))?;
        keys.push(key.clone());
    }
    // Any registered font can stand in for a missing one.
    engine.set_fallback(keys);
    Ok(Some(Arc::new(engine)))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------
