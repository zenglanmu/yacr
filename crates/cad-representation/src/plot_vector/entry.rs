use cad_db::DrawingDatabase;
use cad_domain::{CadResult, LayoutId};

use crate::{
    build_paper_space, plan_plot_for_record, PlotTarget, ProviderRegistry, RepresentationContext,
};

use super::collect::build_vector_document;
use super::pdf::render_pdf;
use super::svg::render_svg;

// ---------------------------------------------------------------------------
// Stable database-facing entry points
// ---------------------------------------------------------------------------

/// Render one paper-space layout to SVG bytes, purely on the CPU.
///
/// This is the stable entry point for vector export: it resolves the layout's
/// [`cad_db::PlotSettingsRecord`], plans the page (honouring margins, rotation
/// and scale), builds the paper-space representation through the provider
/// registry (expanding INSERTs and applying viewport transforms) and serialises
/// SVG. It neither creates nor touches a GPU device.
pub fn render_plot_svg(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    layout: LayoutId,
    target: PlotTarget,
    context: &RepresentationContext,
) -> CadResult<Vec<u8>> {
    let record = database.plot_settings_for(layout);
    let page = plan_plot_for_record(&record, target)?;
    let representation = build_paper_space(registry, database, layout, context, &|_| true)?;
    let document = build_vector_document(&representation, &page)?;
    render_svg(&document)
}

/// Render one paper-space layout to PDF bytes, purely on the CPU.
///
/// See [`render_plot_svg`]; the only difference is the serialisation target.
pub fn render_plot_pdf(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    layout: LayoutId,
    target: PlotTarget,
    context: &RepresentationContext,
) -> CadResult<Vec<u8>> {
    let record = database.plot_settings_for(layout);
    let page = plan_plot_for_record(&record, target)?;
    let representation = build_paper_space(registry, database, layout, context, &|_| true)?;
    let document = build_vector_document(&representation, &page)?;
    render_pdf(&document)
}
