//! Native save, lossy export and plot: the host-owned writing funnels.
//!
//! `SAVE` writes **only** the native `.ymodel` archive (ADR-0041 D1): it is the
//! one lossless, re-openable format, so the opened source's extension never
//! selects a different purpose. `EXPORT` is the explicit "write another format"
//! command: every lossy format goes through the connector registry — DXF, an
//! **experimental** DWG through upstream acadrust, and the neutral-mesh / IFC /
//! SketchUp / glTF lanes — with the **chosen target's extension** deciding the
//! writer. An export may overwrite the opened source (a deliberate user choice);
//! the atomic write's `.bak` is the protection (ADR-0041 D4).
use super::*;
use crate::policy::{self, PlotFormat, SaveFormat};
use cad_domain::Completeness;
use cad_platform::SavePurpose;

pub mod resources;

pub use resources::{
    ResourceCandidate, ResourcePreview, ResourceReport, ResourceSkip, RESOURCE_BUDGET_ENTRY_BYTES,
    RESOURCE_BUDGET_TOTAL_BYTES,
};

/// Outcome of a vector export: how many items the vector document could not
/// represent (unshaped text, unsupported images, ...). A non-zero count means
/// the export is partial and must not be reported as a clean success.
struct PlotOutcome {
    dropped: usize,
}

/// Outcome of a lossy save: the export report's conversion/drop counts, its
/// completeness verdict (the single authority for the graded status) and which
/// format was actually written (decision = target extension, not dialog purpose).
struct SaveOutcome {
    converted: usize,
    dropped: usize,
    completeness: Completeness,
    wrote_dwg: bool,
    /// The registry key that wrote this file when a connector export (STL,
    /// OBJ, PLY, OFF, IFC, SKP) produced it; `None` for the DXF/DWG path.
    connector: Option<&'static str>,
    /// The export wrote a `3DSOLID` into a DXF, so the whole file was written as
    /// AC1024 to keep the ACIS payload as inline SAT text
    /// (`export.note.acis_sat_text_version`). The status names that choice instead
    /// of leaving it in a report the SAVE path never shows.
    acis_sat_ac1024: bool,
    /// The export wrote a `3DSOLID` into a DWG whose ACIS payload is a binary SAB
    /// blob in the AC1032 AcDs section (`export.note.acis_sab_dwg`). The status
    /// names that payload instead of only saying "experimental DWG", and still
    /// states that no third-party reader verified it.
    acis_sab_dwg: bool,
    /// The export wrote a `3DSOLID` into a R2013/R2018 DXF whose ACIS payload is
    /// a binary SAB blob in the `ACDSDATA` section
    /// (`export.note.acis_sab_dxf`). The status names the payload version and the
    /// container instead of only saying "Saved DXF".
    acis_sab_dxf: bool,
}

/// Why a lossy save could not be performed, for a graded status.
enum SaveFailure {
    /// The explicit failure, already localized.
    Reason(String),
}

/// What one native `.ymodel` save did, for the status line.
struct YmodelSaveOutcome {
    /// The recovery snapshot this document had could not be discarded. The file is
    /// durable, so this is a warning, not a failure.
    stale_recovery: Option<String>,
    /// The external resources this save embedded (ADR-0040 D13). Empty when the
    /// document does not embed resources or there was nothing to report.
    resources: ResourceReport,
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
            // The picker-waiting line is the pre-write status the user sees while
            // choosing a target. SAVE and EXPORT are different intents, so each
            // names its own line rather than reusing the vector plot's.
            let waiting = match purpose {
                SavePurpose::Plot => "plot.picker_waiting",
                SavePurpose::SaveExport => "export.picker_waiting",
                // No ymodel-specific catalog line exists yet; the shared
                // "choose where to save" line is the closest existing key.
                SavePurpose::SaveYmodel => "save.picker_waiting",
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

    /// The opened document's source name, when one was successfully opened.
    ///
    /// The platform target recorded at install time is the truthful identity
    /// (SAF URI / browser label / filesystem path); the controller's label hint
    /// is the fallback and also covers the built-in demo document.
    fn save_source_name(&self) -> Option<String> {
        self.source_target
            .borrow()
            .as_ref()
            .map(|target| target.display_name.clone())
    }

    /// The suggested `.ymodel` name for a document: the opened source's stem when
    /// there is one, else the document label hint.
    ///
    /// `SAVE` no longer derives its purpose or default name from the source
    /// extension (ADR-0041 D1): every document saves to the native archive, so the
    /// only thing the source contributes is its stem.
    fn default_ymodel_name(&self, hint: &str) -> String {
        let stem_source = self.save_source_name().unwrap_or_else(|| hint.to_string());
        policy::default_save_name(&stem_source, "ymodel")
    }

    /// The shared "this host can write at all, and is not already busy" guards of
    /// `SAVE`/`EXPORT`/`EXPORT <TOKEN>`.
    ///
    /// The graded status key is the same for all three (`save.export_failed`):
    /// they are one write intent, and the busy/unsupported reason is the real
    /// answer. The caller adds its own additional guards (the ymodel archive for
    /// `SAVE`, nothing for the lossy exports) afterwards.
    fn require_save_ready(&self, command: &str) -> CadResult<()> {
        if !self.capabilities.save {
            let reason = self.message("command.unsupported", &[("command", command)]);
            self.set_catalog_status("save.export_failed", &[("reason", &reason)])?;
            return Err(CadError::Unsupported(reason));
        }
        if self.loading() || self.save_pending.get() || self.pending_save.borrow().is_some() {
            let busy = self.message("linux.busy", &[]);
            self.set_catalog_status("save.export_failed", &[("reason", &busy)])?;
            return Err(CadError::Unsupported(busy));
        }
        Ok(())
    }

    /// Save the active drawing as the native `.ymodel` archive, host-owned.
    ///
    /// `SAVE` is **always** [`SavePurpose::SaveYmodel`] (ADR-0041 D1), whatever
    /// the document was opened from: `.ymodel` is the only lossless, re-openable
    /// format. A carrier without a document archive refuses here with the stable
    /// `unsupported-runtime: ymodel` marker **before** opening a picker — never a
    /// silent export to another format. Every other writing format is reached
    /// through [`Self::request_export`].
    pub(crate) fn request_save(&self) -> CadResult<()> {
        self.require_save_ready("SAVE")?;
        // A carrier with no archive cannot write the native format. Refusing
        // before the picker is the honest answer; opening a chooser that can only
        // fail would look like a save that ran.
        if self.document_archive().is_none() {
            let reason = policy::UNSUPPORTED_YMODEL.to_string();
            self.set_catalog_status("save.export_failed", &[("reason", &reason)])?;
            return Err(CadError::Unsupported(reason));
        }
        let hint = self.controller.borrow().document_name_hint.clone();
        let default_name = self.default_ymodel_name(&hint);
        if let Err(error) = self
            .files
            .begin_save(SavePurpose::SaveYmodel, &default_name)
        {
            self.set_catalog_status("save.export_failed", &[("reason", &error_text(&error))])?;
            return Err(error);
        }
        self.save_pending.set(true);
        self.drain_files()?;
        self.finish_or_wait_save(SavePurpose::SaveYmodel)
    }

    /// Export the active drawing to another format, host-owned (ADR-0041 D2).
    ///
    /// The chooser offers every writable extension and the **chosen target's
    /// extension** decides the writer (the same dispatch `poll_pending_save`
    /// uses for SAVE before this ADR); an unsupported extension keeps its stable
    /// `save.unknown_format` / `<id>.not_implemented` marker. The default name is
    /// the document's stem with the source format's own extension when that
    /// source format is a non-experimental write-back format (ADR-0052), else
    /// `.dxf`; `.dwg`/`.skp` sources therefore still suggest `.dxf`.
    pub(crate) fn request_export(&self) -> CadResult<()> {
        self.require_save_ready("EXPORT")?;
        let hint = self.controller.borrow().document_name_hint.clone();
        let extension =
            policy::export_format_for_source(&hint).map_or("dxf", |spec| spec.extension);
        let default_name = policy::default_save_name(&hint, extension);
        // A bare EXPORT forces no writer: the chosen extension stays the
        // authority. Clearing here drops anything a previous typed request left.
        *self.pending_export_format.borrow_mut() = None;
        if let Err(error) = self
            .files
            .begin_save(SavePurpose::SaveExport, &default_name)
        {
            *self.pending_export_format.borrow_mut() = None;
            self.set_catalog_status("save.export_failed", &[("reason", &error_text(&error))])?;
            return Err(error);
        }
        self.save_pending.set(true);
        self.drain_files()?;
        self.finish_or_wait_save(SavePurpose::SaveExport)
    }

    /// The typed `EXPORT <TOKEN>` form: the same picker, with the chosen writer
    /// fixed for this one interaction (ADR-0052).
    ///
    /// Called by `cad-ui-core` after it resolved the token through
    /// [`policy::export_format_for_token`], so `format_id` is one of the registry
    /// keys and `extension` is that token's own suggested extension. The picker
    /// still runs — the user picks the actual destination — but the chosen
    /// target's extension is still the final authority: a mismatch is refused
    /// before any write by [`Self::export_save`] (`export.format_mismatch`),
    /// never written as a different format.
    pub fn request_export_format(
        &self,
        format_id: &'static str,
        extension: &'static str,
    ) -> CadResult<()> {
        self.require_save_ready("EXPORT")?;
        let hint = self.controller.borrow().document_name_hint.clone();
        let default_name = policy::default_save_name(&hint, extension);
        *self.pending_export_format.borrow_mut() = Some(format_id);
        if let Err(error) = self
            .files
            .begin_save(SavePurpose::SaveExport, &default_name)
        {
            *self.pending_export_format.borrow_mut() = None;
            self.set_catalog_status("save.export_failed", &[("reason", &error_text(&error))])?;
            return Err(error);
        }
        self.save_pending.set(true);
        self.drain_files()?;
        self.finish_or_wait_save(SavePurpose::SaveExport)
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
        // A typed `EXPORT <TOKEN>` armed the writer for exactly this interaction.
        // Taking it here covers the target, the cancel and the platform failure
        // alike, so a forced writer never leaks into a later save; a bare EXPORT
        // and every other purpose leave the slot `None`.
        let forced_export = if purpose == SavePurpose::SaveExport {
            self.pending_export_format.borrow_mut().take()
        } else {
            None
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
            // A closed dialog is a cancel, not a failure (all save formats).
            (SavePurpose::SaveExport | SavePurpose::SaveYmodel, Err(FileAbort::Cancelled)) => {
                catalog("save.cancelled", &[])
            }
            (SavePurpose::Plot, Err(FileAbort::Cancelled)) => catalog("plot.cancelled", &[]),
            // The platform's reason is already localized: show it verbatim.
            (SavePurpose::SaveExport | SavePurpose::SaveYmodel, Err(FileAbort::Failed(reason))) => {
                catalog("save.export_failed", &[("reason", &reason)])
            }
            (SavePurpose::Plot, Err(FileAbort::Failed(reason))) => {
                catalog("plot.export_failed", &[("reason", &reason)])
            }
            (SavePurpose::SaveExport, Ok(target)) => {
                match self.export_save(&target, forced_export) {
                    // A connector export (mesh/IFC/SKP) has its own wording: the
                    // file is not a DXF, and the format key is what the user
                    // chose. Complete vs. partial still comes from the report's
                    // single completeness authority.
                    Ok(outcome) if outcome.connector.is_some() => {
                        let format = outcome.connector.unwrap_or_default();
                        if outcome.completeness == Completeness::Complete {
                            catalog(
                                "save.exported_connector",
                                &[("path", &target.display_name), ("format", format)],
                            )
                        } else {
                            catalog(
                                "save.exported_connector_partial",
                                &[
                                    ("path", &target.display_name),
                                    ("format", format),
                                    ("converted", &outcome.converted.to_string()),
                                    ("dropped", &outcome.dropped.to_string()),
                                ],
                            )
                        }
                    }
                    // A DWG write always states it is experimental (the target
                    // extension, not the opened source, decided this lane); a
                    // clean/partial distinction only changes whether conversions
                    // are counted. A DWG that carries an exact `3DSOLID` says so
                    // too (`save.exported_acis_sab`), because its ACIS payload is a
                    // SAB blob in the AcDs section.
                    Ok(outcome) if outcome.wrote_dwg => {
                        let clean = outcome.completeness == Completeness::Complete;
                        let key = match (outcome.acis_sab_dwg, clean) {
                            (true, true) => "save.exported_acis_sab",
                            (true, false) => "save.exported_acis_sab_partial",
                            (false, true) => "save.dwg_exported_experimental",
                            (false, false) => "save.dwg_exported_partial",
                        };
                        if clean {
                            catalog(key, &[("path", &target.display_name)])
                        } else {
                            catalog(
                                key,
                                &[
                                    ("path", &target.display_name),
                                    ("converted", &outcome.converted.to_string()),
                                    ("dropped", &outcome.dropped.to_string()),
                                ],
                            )
                        }
                    }
                    // Clean success only when the export report is Complete; the
                    // UI never re-derives the verdict from the counts. A file that
                    // carries a `3DSOLID` says which container/payload combination
                    // was written (R2010 inline SAT text, or a R2013/R2018 SAB
                    // blob in ACDSDATA), which the status states out loud.
                    Ok(outcome) if outcome.completeness == Completeness::Complete => {
                        let key = if outcome.acis_sab_dxf {
                            "save.exported_acis_sab_dxf"
                        } else if outcome.acis_sat_ac1024 {
                            "save.exported_acis_sat"
                        } else {
                            "save.exported"
                        };
                        catalog(key, &[("path", &target.display_name)])
                    }
                    Ok(outcome) => {
                        let key = if outcome.acis_sab_dxf {
                            "save.exported_acis_sab_dxf_partial"
                        } else if outcome.acis_sat_ac1024 {
                            "save.exported_acis_sat_partial"
                        } else {
                            "save.exported_partial"
                        };
                        catalog(
                            key,
                            &[
                                ("path", &target.display_name),
                                ("converted", &outcome.converted.to_string()),
                                ("dropped", &outcome.dropped.to_string()),
                            ],
                        )
                    }
                    Err(SaveFailure::Reason(reason)) => {
                        catalog("save.export_failed", &[("reason", &reason)])
                    }
                }
            }
            (SavePurpose::SaveYmodel, Ok(target)) => match self.export_ymodel(&target) {
                // The native format has its own wording: reusing `save.exported`
                // would tell the user a DXF was written when the file is a
                // `.ymodel`. The foreign-entry count has no key yet, so it stays
                // in the document's persistence state rather than being guessed at.
                //
                // A save that packed external resources reports them on this same
                // line (ADR-0040 D13): the catalog's two sentences
                // (`save.embed.report` / `save.embed.report_skipped`) followed by
                // the machine detail when anything was skipped or failed. It is
                // deliberately *not* written into `tables/compat.json` — that table
                // describes the content and is capped at 32 rows, so an over-budget
                // resource would be lost.
                Ok(outcome) if !outcome.resources.is_empty() => StatusMessage::ExternalText(
                    outcome
                        .resources
                        .status_text(&target.display_name, outcome.stale_recovery.as_deref()),
                ),
                Ok(YmodelSaveOutcome {
                    resources: _,
                    stale_recovery: None,
                }) => catalog("save.exported_ymodel", &[("path", &target.display_name)]),
                // Saved, but the recovery snapshot could not be removed. The file
                // is durable, so this is a warning that names both facts instead
                // of failing a save that succeeded.
                Ok(YmodelSaveOutcome {
                    resources: _,
                    stale_recovery: Some(reason),
                }) => catalog(
                    "save.exported_ymodel_stale_recovery",
                    &[("path", &target.display_name), ("reason", &reason)],
                ),
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

    /// Write the lossy drawing export for `target`'s extension (the `EXPORT`
    /// command, ADR-0041 D2/D4).
    ///
    /// Every extension — `.dxf`, `.dwg` and the connector keys
    /// `.stl`/`.obj`/`.ply`/`.off`/`.coff`/`.ifc`/`.skp` — is dispatched through
    /// the connector registry ([`HostController::export_connector`]), so the
    /// writer's own [`cad_connector_core::CompatReport`] is the single source of the
    /// counts, the verdict and the note codes. Any other extension fails.
    ///
    /// **Overwriting is allowed** (ADR-0041 D4): a target that names the opened
    /// source is a deliberate user choice, not a refusal. The previous bytes are
    /// protected by the platform's atomic write (the Linux carrier keeps a
    /// `<name>.<ext>.bak`); the host adds no identity guard of its own.
    ///
    /// After the bytes are durable, the report is recorded in the in-memory
    /// `Document.compat` table (the next `.ymodel` save persists it). A recording
    /// failure never fails the save: the file is already written.
    ///
    /// `forced` is the writer a typed `EXPORT <TOKEN>` fixed for this interaction
    /// (ADR-0052). The classification below is still derived from the **chosen
    /// target's extension**, which remains the authority: a forced writer that
    /// disagrees with it is refused as `export.format_mismatch` **before** any
    /// bytes are produced or written, never silently written as the other format.
    fn export_save(
        &self,
        target: &FileTarget,
        forced: Option<&'static str>,
    ) -> Result<SaveOutcome, SaveFailure> {
        let (wrote_dwg, format_id): (bool, &'static str) =
            match policy::save_format(&policy::extension_of(&target.display_name)) {
                SaveFormat::Dwg => (true, "dwg"),
                SaveFormat::Dxf => (false, "dxf"),
                // Every other supported extension is a connector export key
                // (`stl`, `obj`, `ply`, `off`, `coff`, `ifc`, `skp`).
                SaveFormat::Connector(id) => (false, id),
                SaveFormat::NoExtension => {
                    return Err(SaveFailure::Reason(self.message("save.no_extension", &[])))
                }
                SaveFormat::Unknown(ext) => {
                    return Err(SaveFailure::Reason(
                        self.message("save.unknown_format", &[("ext", &ext)]),
                    ))
                }
            };
        if let Some(requested) = forced {
            // The extension, not the token, decides: `.coff` classifies to the
            // `off` writer, so `EXPORT COFF` matched against `.coff` or `.off`
            // both succeed while `EXPORT GLB` against `.dxf` is refused.
            if requested != format_id {
                return Err(SaveFailure::Reason(format!(
                    "export.format_mismatch: requested {requested}, target .{} is {format_id}",
                    policy::extension_of(&target.display_name)
                )));
            }
        }
        // Install the configured Save-As options on the controller first: the
        // ACIS payload version and DXF container are host settings, and the DXF
        // and DWG lanes read them when they build their request. The mesh/IFC/SKP
        // writers ignore them. A DWG is always AC1032, so the registry names the
        // DWG's own container for that format.
        let options = self.export_options();
        self.controller.borrow_mut().set_export_options(options);
        // The connector lane: the registry is the dispatch authority (DXF/DWG
        // included) and the writer's own report supplies both the counts and the
        // verdict.
        let output = self
            .controller
            .borrow()
            .export_connector(format_id)
            .map_err(|error| SaveFailure::Reason(error_text(&error)))?;
        self.files
            .write_atomic(target, &output.bytes)
            .map_err(|error| SaveFailure::Reason(error_text(&error)))?;
        // The file is durable; recording the report is metadata. A failure here
        // (no document open, a poisoned slot) must not turn a written file into a
        // failed save, so it is deliberately ignored. The next `.ymodel` save
        // persists whatever was recorded.
        let _ = self
            .controller
            .borrow_mut()
            .record_export_report(&output.report);
        let dxf_or_dwg = format_id == "dxf" || format_id == "dwg";
        Ok(SaveOutcome {
            converted: output.report.counts.converted,
            dropped: output.report.counts.dropped,
            completeness: output.report.completeness,
            wrote_dwg,
            connector: if dxf_or_dwg { None } else { Some(format_id) },
            // The ACIS notes are produced by the acadrust DXF/DWG adapter and
            // copied verbatim into the shared report; no other lane writes them.
            acis_sat_ac1024: output
                .report
                .entries
                .iter()
                .any(|entry| entry.code == "export.note.acis_sat_text_version"),
            acis_sab_dwg: output
                .report
                .entries
                .iter()
                .any(|entry| entry.code == "export.note.acis_sab_dwg"),
            acis_sab_dxf: output
                .report
                .entries
                .iter()
                .any(|entry| entry.code == "export.note.acis_sab_dxf"),
        })
    }

    /// Write the native `.ymodel` archive for `target` (ADR-0037 D8/D11/D12).
    ///
    /// The order is the contract:
    /// 1. the carrier archive must exist — otherwise the marker, never a fallback
    ///    export of another format;
    /// 2. `previous` is the document's **own** archive bytes
    ///    ([`crate::PersistenceState::archive_bytes`]), not the target's:
    ///    a narrowed write keeps that base's entries and a Save As must not adopt
    ///    a different file's contents;
    /// 3. for an **in-place** save only, re-read the target and refuse when its
    ///    bytes differ from what was opened/saved (`ymodel.conflict`) — the
    ///    no-lock-file rule of D12. An unreadable in-place target is a refusal
    ///    too, because the write would replace bytes it never verified. A Save As
    ///    to a different/new target reads nothing;
    /// 4. `dirty` is the scope accumulated by every commit since the last
    ///    save/open (empty for a first save, which conservatively rewrites);
    /// 5. encode from a **fresh snapshot of the current revision**;
    /// 6. write atomically, and only then advance `saved_revision`, retain the
    ///    committed bytes as the next base and clear the dirty scope. A write
    ///    failure leaves the state untouched.
    /// 7. discard the recovery snapshot this document was writing.
    ///
    /// Step 7 is the only recovery action a save takes, and it never fails the
    /// save: the bytes are already durable, and a snapshot that survives can only
    /// be discarded as stale at the next open (it is never offered above the
    /// saved revision). Its refusal is returned as a warning for the status line,
    /// not as an error.
    ///
    /// Between steps 4 and 5 the external resources are collected when the
    /// document opted into self-containment (ADR-0040 D13): the collection writes
    /// the decided bytes into `cad-db` (so the snapshot below is the authority),
    /// and **never fails the save** — a missing or over-budget resource is a
    /// report, not an error.
    fn export_ymodel(&self, target: &FileTarget) -> Result<YmodelSaveOutcome, SaveFailure> {
        let Some(archive) = self.document_archive() else {
            // A carrier with no archive cannot save the native format. Explicit,
            // never a silent DXF/DWG export under a `.ymodel` name.
            return Err(SaveFailure::Reason(policy::UNSUPPORTED_YMODEL.to_string()));
        };
        // In-place means the target is the exact handle the host recorded for the
        // opened/last-saved file (`source_target` is always set from a real
        // `FileTarget`, never re-derived), so the opaque handle is the identity.
        let in_place = self
            .source_target
            .borrow()
            .as_ref()
            .is_some_and(|source| source.opaque == target.opaque);
        // The key the snapshot was written under, captured **before** the write
        // moves `source_target` (a first save switches uuid key → path key, and a
        // Save As switches to a new path): otherwise the discard would look under
        // the new name and leave the old snapshot behind.
        let recovery_key = self.recovery_key();
        let persistence = self.controller.borrow().persistence().cloned();
        // Conflict detection applies only to the file this document was opened
        // from: a Save As to a different, existing archive must not refuse because
        // its bytes differ from the source we opened, and must not need to read it
        // at all.
        if in_place {
            let target_bytes = self
                .files
                .read_target(target)
                // An existing in-place target we cannot re-read is a real
                // refusal, not "absent": the save would overwrite a file whose
                // contents it never verified.
                .map_err(SaveFailure::Reason)?;
            if let Some(expected) = persistence
                .as_ref()
                .and_then(|state| state.source_sha256.clone())
            {
                if sha256_hex(&target_bytes) != expected {
                    return Err(SaveFailure::Reason(
                        cad_domain::ymodel::markers::CONFLICT.to_string(),
                    ));
                }
            }
        }
        // The base of the write: the document's own archive, never the target's
        // bytes. `None` (blank/imported) means there is nothing to copy through.
        let previous = persistence
            .as_ref()
            .and_then(|state| state.archive_bytes.clone());
        // ADR-0040 D13: collect the document's external resources **before** the
        // snapshot, so the bytes the write encodes are already `cad-db` state and
        // the scope it computes names `tables/resources.json`. A no-op for every
        // document that never opted in (the flag is a document property, D3).
        let resources = self.collect_ymodel_resources();
        let snapshot = self
            .controller
            .borrow()
            .snapshot_document()
            .map_err(|error| SaveFailure::Reason(error_text(&error)))?;
        // Every commit since the last save/open, accumulated by the controller
        // (command path, plugin commits and the engine request funnel). Empty for
        // a first save, where rewriting every entry is the conservative answer.
        let dirty = self.controller.borrow().dirty_scope().clone();
        let document_uuid = persistence.as_ref().and_then(|p| p.document_uuid.clone());
        let open_revision = persistence
            .as_ref()
            .and_then(|p| p.open_revision)
            // No baseline: `u64::MAX` makes the L0 passthrough predicate
            // unsatisfiable rather than accidentally true (D2).
            .map_or(u64::MAX, |revision| revision.0);
        let request = cad_platform::ArchiveWrite {
            snapshot: &snapshot,
            dirty: &dirty,
            previous: previous.as_deref(),
            document_uuid: document_uuid.as_deref(),
            open_revision,
            // No clock in the shared host: the manifest timestamp stays absent.
            saved_at: None,
        };
        let encoded = archive
            .write(&request)
            .map_err(|error| SaveFailure::Reason(format!("{}: {}", error.marker, error.detail)))?;
        self.files
            .write_atomic(target, &encoded.bytes)
            .map_err(|error| SaveFailure::Reason(error_text(&error)))?;
        // Durable: now the document may claim the new saved revision, retain the
        // bytes just written and clear the scope that produced them — one fact,
        // committed only after the write.
        let committed: Arc<[u8]> = Arc::from(encoded.bytes);
        let source_sha256 = sha256_hex(&committed);
        // The archive does not echo the uuid it may have minted, so this either
        // keeps the known identity or stays `None` — never a fabricated v4 uuid.
        let foreign_entries = persistence
            .as_ref()
            .map(|state| state.foreign_entries.clone())
            .unwrap_or_default();
        {
            let mut controller = self.controller.borrow_mut();
            controller.mark_saved(encoded.saved_revision, document_uuid, foreign_entries);
            controller.set_source_sha256(Some(source_sha256));
            controller.record_saved_archive(committed);
        }
        // The document now *is* this file: the next save is in place against it
        // and the source-overwrite guard must name the target we just wrote.
        *self.source_target.borrow_mut() = Some(target.clone());
        self.files.set_source(Some(target));
        self.full_dirty.set(true);
        // The document is durable: the snapshot it was protecting is obsolete.
        // A refusal here is surfaced on the save status by the caller, but it
        // never turns a written file into a failed save.
        Ok(YmodelSaveOutcome {
            stale_recovery: self.discard_recovery(recovery_key.as_ref()),
            resources,
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
    pub(crate) fn plot_context(&self) -> CadResult<cad_representation::RepresentationContext> {
        let (document, generation, requested, text_needed, wipeout_frames, fill) = {
            let controller = self.controller.borrow();
            (
                controller.document_id,
                controller.session.generation,
                controller
                    .drawing()
                    .map(|drawing| cad_platform::fonts::requested_fonts(drawing.as_ref()))
                    .unwrap_or_default(),
                controller
                    .drawing()
                    .map(|drawing| cad_platform::fonts::needs_text_shaping(drawing.as_ref()))
                    .unwrap_or(false),
                controller.session.drafting_extras.wipeout_frames(),
                controller.session.drafting_extras.fill(),
            )
        };
        let mut context = cad_representation::RepresentationContext::new(
            document,
            cad_domain::TolerancePolicy::default(),
            cad_domain::TaskStamp::new(document, generation),
        );
        // `WIPEOUTFRAME 1` plots the boundary frame; `2` displays it only and
        // `0` hides it. The vector path therefore builds its context with the
        // frame flag set **only for ON** — this is the whole display/plot split,
        // and it never writes the drawing.
        context = context.with_wipeout_frames(wipeout_frames == cad_app::WipeoutFrames::On);
        // `FILL` governs the display *and* the vector output: `FILL OFF` plots
        // filled hatches/2D solids/wide polylines as their outline, the same rule
        // the display base applies. `QTEXT` is deliberately not applied here —
        // quick text is a screen mode and AutoCAD plots the real text.
        context = context.with_fill(fill);
        if let Some(engine) = self.load_plot_fonts(&requested, text_needed)? {
            context = context.with_fonts(engine);
        }
        Ok(context)
    }
}
