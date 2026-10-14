//! Plot (SVG/PDF) and lossy DXF Save As: the host-owned export funnel.
use super::*;
use crate::policy::{self, PlotFormat, SaveFormat};
use cad_domain::Completeness;
use cad_platform::SavePurpose;

/// Outcome of a vector export: how many items the vector document could not
/// represent (unshaped text, unsupported images, ...). A non-zero count means
/// the export is partial and must not be reported as a clean success.
struct PlotOutcome {
    dropped: usize,
}

/// Outcome of a lossy DXF save: the export report's conversion/drop counts and
/// its completeness verdict (the single authority for the graded status).
struct SaveOutcome {
    converted: usize,
    dropped: usize,
    completeness: Completeness,
}

/// Why a lossy save could not be performed, for a graded status.
enum SaveFailure {
    /// The chosen target is the opened source file (data-corruption guard).
    RefusedSourcePath,
    /// `.dwg` output is not verified yet.
    DwgDeferred,
    /// Any other explicit failure, already localized.
    Reason(String),
}

impl HostCore {
    /// Export the active drawing to a vector file (SVG/PDF), host-owned.
    ///
    /// The save path comes from the platform. The format is decided by the
    /// chosen extension: `.svg`/`.pdf` render on the CPU vector path; `.png` is
    /// an explicit refusal (GUI PNG needs surface readback, which the CLI does);
    /// anything else fails.
    pub(crate) fn request_plot(&self) -> CadResult<()> {
        if !self.capabilities.plot {
            let reason = self.message("command.unsupported", &[("command", "PLOT")]);
            self.report_plot_failed(&reason)?;
            return Err(CadError::Unsupported(reason));
        }
        if self.loading() || self.save_pending.get() || self.pending_save.borrow().is_some() {
            let busy = self.message("linux.busy", &[]);
            self.report_plot_failed(&busy)?;
            return Err(CadError::Unsupported(busy));
        }
        let hint = self.controller.borrow().document_name_hint.clone();
        // The vector format comes from the validated host config
        // (`ui.export.plotFormat`, default svg): on the desktop it only picks the
        // dialog's suggested extension (the user still types the real one); on
        // the web, which has no dialog, it is authoritative for the download.
        let default_name = policy::default_save_name(&hint, self.plot_export_format().as_str());
        if let Err(error) = self.files.begin_save(SavePurpose::Plot, &default_name) {
            self.report_plot_failed(&error_text(&error))?;
            return Err(error);
        }
        self.save_pending.set(true);
        self.drain_files()?;
        self.finish_or_wait_save(SavePurpose::Plot)
    }

    /// Finish a save inside the gesture when the platform is synchronous.
    ///
    /// A desktop picker leaves `save_pending` set (the timer drains the chosen
    /// target later) and shows the picker-waiting status. A synchronous platform
    /// (the browser synthesizes the download target in `begin_save`) clears the
    /// flag during [`Self::drain_files`], so the export must run now to keep the
    /// whole download in the same gesture as the click.
    fn finish_or_wait_save(&self, purpose: SavePurpose) -> CadResult<()> {
        if self.save_pending.get() {
            let waiting = match purpose {
                SavePurpose::Plot => "plot.picker_waiting",
                SavePurpose::SaveDxf => "save.picker_waiting",
            };
            self.set_catalog_status(waiting, &[])?;
            return Ok(());
        }
        self.poll_pending_save()
    }

    /// The host-configured default vector plot format (`ui.export.plotFormat`).
    ///
    /// Read the effective format from the host-owned config store.
    fn plot_export_format(&self) -> cad_app::viewer_config::PlotExportFormat {
        self.viewer_config
            .borrow()
            .effective()
            .ui
            .export
            .plot_format
    }

    /// Save the active drawing as a lossy DXF file, host-owned.
    ///
    /// `.dxf` uses the lossy export core; `.dwg` is an explicit refusal (DWG
    /// write is not verified yet).
    pub(crate) fn request_save(&self) -> CadResult<()> {
        if !self.capabilities.save {
            let reason = self.message("command.unsupported", &[("command", "SAVE")]);
            self.set_catalog_status("save.export_failed", &[("reason", &reason)])?;
            return Err(CadError::Unsupported(reason));
        }
        if self.loading() || self.save_pending.get() || self.pending_save.borrow().is_some() {
            let busy = self.message("linux.busy", &[]);
            self.set_catalog_status("save.export_failed", &[("reason", &busy)])?;
            return Err(CadError::Unsupported(busy));
        }
        let hint = self.controller.borrow().document_name_hint.clone();
        let default_name = policy::default_save_name(&hint, "dxf");
        if let Err(error) = self.files.begin_save(SavePurpose::SaveDxf, &default_name) {
            self.set_catalog_status("save.export_failed", &[("reason", &error_text(&error))])?;
            return Err(error);
        }
        self.save_pending.set(true);
        self.drain_files()?;
        self.finish_or_wait_save(SavePurpose::SaveDxf)
    }

    /// Drain a finished save interaction and perform the requested export.
    ///
    /// Runs on the timer; it always sets an explicit status and never returns an
    /// error the timer would turn into a generic failure. A cancel writes
    /// nothing and reports the cancel, not a failure.
    pub(crate) fn poll_pending_save(&self) -> CadResult<()> {
        let Some(PendingSave { purpose, result }) = self.pending_save.borrow_mut().take() else {
            return Ok(());
        };
        let catalog = |key: &str, args: &[(&str, &str)]| StatusMessage::Catalog {
            key: key.to_string(),
            args: args
                .iter()
                .map(|(name, value)| StatusArgument {
                    key: (*name).to_string(),
                    value: (*value).to_string(),
                })
                .collect(),
        };
        let status = match (purpose, result) {
            // A closed dialog is a cancel, not a failure.
            (SavePurpose::SaveDxf, Err(FileAbort::Cancelled)) => catalog("save.cancelled", &[]),
            (SavePurpose::Plot, Err(FileAbort::Cancelled)) => catalog("plot.cancelled", &[]),
            // The platform's reason is already localized: show it verbatim.
            (SavePurpose::SaveDxf, Err(FileAbort::Failed(reason))) => {
                catalog("save.export_failed", &[("reason", &reason)])
            }
            (SavePurpose::Plot, Err(FileAbort::Failed(reason))) => {
                catalog("plot.export_failed", &[("reason", &reason)])
            }
            (SavePurpose::SaveDxf, Ok(target)) => match self.export_save(&target) {
                // Clean success only when the export report is Complete; the UI
                // never re-derives the verdict from the counts.
                Ok(outcome) if outcome.completeness == Completeness::Complete => {
                    catalog("save.exported", &[("path", &target.display_name)])
                }
                Ok(outcome) => catalog(
                    "save.exported_partial",
                    &[
                        ("path", &target.display_name),
                        ("converted", &outcome.converted.to_string()),
                        ("dropped", &outcome.dropped.to_string()),
                    ],
                ),
                Err(SaveFailure::RefusedSourcePath) => catalog("save.refused_source_path", &[]),
                Err(SaveFailure::DwgDeferred) => catalog("save.dwg_deferred", &[]),
                Err(SaveFailure::Reason(reason)) => {
                    catalog("save.export_failed", &[("reason", &reason)])
                }
            },
            (SavePurpose::Plot, Ok(target)) => match self.export_plot(&target) {
                // Clean success only when nothing was dropped; a partial export
                // is reported as such, never as a clean success.
                Ok(outcome) if outcome.dropped == 0 => {
                    catalog("plot.exported", &[("path", &target.display_name)])
                }
                Ok(outcome) => catalog(
                    "plot.exported_partial",
                    &[
                        ("path", &target.display_name),
                        ("count", &outcome.dropped.to_string()),
                    ],
                ),
                Err(error) => catalog("plot.export_failed", &[("reason", &error_text(&error))]),
            },
        };
        self.set_status_message(status)?;
        Ok(())
    }

    /// Write the lossy DXF for `target`'s extension.
    ///
    /// `.dxf` → the export core; `.dwg` → explicit refusal; any other extension
    /// fails. Refuses to overwrite the opened source file.
    fn export_save(&self, target: &FileTarget) -> Result<SaveOutcome, SaveFailure> {
        match policy::save_format(&policy::extension_of(&target.display_name)) {
            SaveFormat::Dwg => return Err(SaveFailure::DwgDeferred),
            SaveFormat::NoExtension => {
                return Err(SaveFailure::Reason(self.message("save.no_extension", &[])))
            }
            SaveFormat::Unknown(ext) => {
                return Err(SaveFailure::Reason(
                    self.message("save.unknown_format", &[("ext", &ext)]),
                ))
            }
            SaveFormat::Dxf => {}
        }
        if self.files.refuses_source_overwrite(target) {
            return Err(SaveFailure::RefusedSourcePath);
        }
        let (bytes, report) = self
            .controller
            .borrow()
            .export_dxf()
            .map_err(|error| SaveFailure::Reason(error_text(&error)))?;
        self.files
            .write_atomic(target, &bytes)
            .map_err(|error| SaveFailure::Reason(error_text(&error)))?;
        Ok(SaveOutcome {
            converted: report.counts.converted,
            dropped: report.counts.dropped,
            completeness: report.completeness,
        })
    }

    fn report_plot_failed(&self, reason: &str) -> CadResult<()> {
        self.set_catalog_status("plot.export_failed", &[("reason", reason)])?;
        Ok(())
    }

    /// Render and atomically write the vector export selected by `target`'s
    /// extension. Never writes a vector document into a `.png` name.
    fn export_plot(&self, target: &FileTarget) -> CadResult<PlotOutcome> {
        let extension = policy::extension_of(&target.display_name);
        match policy::plot_format(&extension) {
            PlotFormat::Png => {
                return Err(CadError::Unsupported(
                    self.message("plot.png_cli_only", &[]),
                ))
            }
            PlotFormat::NoExtension => {
                return Err(CadError::InvalidInput(
                    self.message("plot.no_extension", &[]),
                ))
            }
            PlotFormat::Unknown(ext) => {
                return Err(CadError::InvalidInput(
                    self.message("plot.unknown_format", &[("ext", &ext)]),
                ))
            }
            PlotFormat::Svg | PlotFormat::Pdf => {}
        }
        let (database, layout) = self.plot_layout()?;
        let registry = cad_representation::ProviderRegistry::with_default_provider();
        let context = self.plot_context()?;
        let record = database.plot_settings_for(layout);
        // 96 DPI is a nominal vector resolution; SVG/PDF carry the physical page
        // size, not the raster size.
        let page = cad_representation::plan_plot_for_record(
            &record,
            cad_representation::PlotTarget::Dpi(96.0),
        )?;
        let representation =
            cad_representation::build_paper_space(&registry, &database, layout, &context, &|_| {
                true
            })?;
        let document = cad_representation::build_vector_document(&representation, &page)?;
        let dropped = document.diagnostics.len();
        let bytes = if extension == "svg" {
            cad_representation::render_svg(&document)?
        } else {
            cad_representation::render_pdf(&document)?
        };
        if bytes.is_empty() {
            return Err(CadError::InvalidInput(
                self.message("plot.empty_output", &[]),
            ));
        }
        self.files.write_atomic(target, &bytes)?;
        Ok(PlotOutcome { dropped })
    }

    /// The database clone and default layout to plot: the sheet with the most
    /// viewports, else the first, excluding the synthesised model-space
    /// `LayoutId(0)` "Model" (model space is not a sheet).
    ///
    /// Drift risk (P6): the CLI keeps an equivalent `select_plot_layout`
    /// (`crates/cad-cli-tools/src/ops.rs`). The two must stay behaviourally
    /// identical; a shared helper would need to live in `cad-representation`.
    fn plot_layout(&self) -> CadResult<(cad_db::DrawingDatabase, cad_domain::LayoutId)> {
        let drawing = self
            .controller
            .borrow()
            .drawing()
            .ok_or_else(|| CadError::InvalidInput(self.message("plot.no_document", &[])))?;
        let descriptors: Vec<_> = cad_representation::enumerate_layouts(drawing.as_ref())
            .into_iter()
            .filter(|d| !(d.id == cad_domain::LayoutId(0) && d.name == "Model"))
            .collect();
        if descriptors.is_empty() {
            return Err(CadError::InvalidInput(self.message("plot.no_layout", &[])));
        }
        let id = match descriptors.iter().max_by_key(|d| d.viewport_count) {
            Some(richest) if richest.viewport_count > 0 => richest.id,
            _ => descriptors[0].id,
        };
        Ok(((*drawing).clone(), id))
    }

    /// The representation context for plotting, with the desktop font engine so
    /// text is shaped exactly as the render path shapes it.
    fn plot_context(&self) -> CadResult<cad_representation::RepresentationContext> {
        let (document, generation, requested) = {
            let controller = self.controller.borrow();
            (
                controller.document_id,
                controller.session.generation,
                controller
                    .drawing()
                    .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
                    .unwrap_or_default(),
            )
        };
        let mut context = cad_representation::RepresentationContext::new(
            document,
            cad_domain::TolerancePolicy::default(),
            cad_domain::TaskStamp::new(document, generation),
        );
        if let Some(engine) = self.load_plot_fonts(&requested)? {
            context = context.with_fonts(engine);
        }
        Ok(context)
    }
}
