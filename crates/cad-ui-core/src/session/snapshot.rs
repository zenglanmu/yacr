//! The `cad_status` snapshot assembler.
//!
//! Split out of `session/mod.rs` for the code-size gate. The public path is
//! unchanged (`CadSession::snapshot`); this module only reads already-derived
//! state and never mutates the database, history or a preference.

use super::*;

impl CadSession {
    /// Build the frontend status snapshot from the current authoritative state.
    ///
    /// `backend`/`adapter`/`frame_count` are the GPU lane's actual values. The
    /// drawing label, entity count and camera come from the authoritative
    /// controller document/viewport; an unknown value stays `null` (a 3D or
    /// non-orthographic view has no 2D camera block, and a missing document has
    /// no counts or camera).
    ///
    /// This is the sole host-update consumer in this host, so it also harvests
    /// the host's projected resource state ([`HostCore::take_updates`]) to
    /// report missing/partial images; a partial resource set is surfaced as a
    /// technical `error`, never as a silent "complete".
    /// Take the vendor records copied by the last successful import.
    ///
    /// Transport state, not document state (`Some([])` = a new document with no
    /// vendor objects; `None` = nothing new).
    pub fn take_vendor_records(&self) -> Option<Vec<cad_host::VendorRecord>> {
        self.core.take_vendor_records()
    }

    pub fn snapshot(
        &self,
        backend: Option<String>,
        adapter: Option<String>,
        frame_count: u64,
    ) -> CadStatus {
        self.observe_import();
        self.observe_resources();
        let (drawing, label, camera, selection, layouts, active_space, view) = {
            let c = self.core.controller.borrow();
            let drawing = c.drawing();
            let label = c.document_name_hint.clone();
            let camera = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .and_then(|viewport| viewport.camera.camera2d_params().ok())
                .and_then(camera_status);
            // The real current selection, read from the authoritative
            // controller each time the snapshot is built. Without a document
            // there is no property source, so only the real ref count travels
            // (never an omitted block and never fabricated rows).
            let selection = c
                .selection_properties()
                .map(|properties| CadSelection::from_properties(&properties))
                .unwrap_or_else(|_| CadSelection::refs_only(c.selection().len()));
            // One tab per **paper-space** layout, in the authoritative
            // descriptor order. The synthetic `LayoutId(0)` "Model" descriptor
            // is filtered out: model space is its own tab, represented by
            // `CadActiveSpace::Model`, not a paper sheet.
            let layouts: Vec<CadLayoutTab> = drawing
                .as_ref()
                .map(|database| {
                    cad_app::render_scene::layout_descriptors(database.as_ref())
                        .into_iter()
                        .filter(|descriptor| descriptor.id.0 != 0)
                        .map(|descriptor| {
                            CadLayoutTab::new(
                                descriptor.id.0,
                                descriptor.name,
                                descriptor.supported,
                                descriptor.reason,
                                descriptor.viewport_count,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            // The session's real active space through the controller's pure
            // getter — the one projection point, never a UI-side "current tab".
            let active_space = CadActiveSpace::from_space_id(&c.active_space());
            // The current view's real orientation: the host's observation mode,
            // the camera's real projection and its world basis. Only a fully
            // finite basis is reported; otherwise `null` (never a fabricated
            // orientation). A 2D plan view is a real orthographic camera, so it
            // reports its basis like any other view.
            let view = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .and_then(|viewport| {
                    let finite = |point: cad_domain::Point3| {
                        point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
                    };
                    let camera = viewport.camera;
                    if !(finite(camera.eye) && finite(camera.target) && finite(camera.up)) {
                        return None;
                    }
                    let mode = match viewport.view_mode.kind() {
                        cad_app::ProjectionKind::TwoD => "2d",
                        cad_app::ProjectionKind::ThreeD => "3d",
                    };
                    let projection = match camera.projection {
                        cad_app::Projection::Orthographic { .. } => "orthographic",
                        cad_app::Projection::Perspective { .. } => "perspective",
                    };
                    Some(CadViewOrientation {
                        mode: mode.to_string(),
                        projection: projection.to_string(),
                        eye: [camera.eye.x, camera.eye.y, camera.eye.z],
                        target: [camera.target.x, camera.target.y, camera.target.z],
                        up: [camera.up.x, camera.up.y, camera.up.z],
                    })
                });
            (
                drawing,
                label,
                camera,
                selection,
                layouts,
                active_space,
                view,
            )
        };
        // The host's persisted theme and navigation switches, read from the one
        // settings projection (never a second derivation).
        let settings = self.core.settings();
        let navigation = CadNavigation {
            navbar_visible: settings.navbar_visible,
            view_cube_display: settings.navvcube_display,
            view_cube_in_2d: settings.view_cube_in_2d,
            view_cube_in_3d: settings.view_cube_in_3d,
        };
        let theme = settings.theme;
        let has_document = drawing.is_some();
        // No authoritative document means no trustworthy view: report `null`
        // rather than a camera basis that belongs to no drawing.
        let view = if has_document { view } else { None };
        // The authoritative xref metadata, read in the drawing's stable
        // reference-name (`BTreeMap`) order. No document means no references;
        // a reference that could not be loaded keeps its real status/reason.
        let references: Vec<CadReferenceStatus> = drawing
            .as_ref()
            .map(|database| {
                database
                    .external_references()
                    .map(CadReferenceStatus::from_reference)
                    .collect()
            })
            .unwrap_or_default();
        // The classic-panel projections: authoritative block definitions (with
        // stored metadata when present), the session's own group store, and the
        // IMAGE entities in every space. No document means honest empty lists.
        let blocks: Vec<CadBlockStatus> = drawing
            .as_deref()
            .map(|database| {
                database
                    .blocks()
                    .map(|definition| {
                        CadBlockStatus::from_definition(
                            definition,
                            database.block_metadata(definition.id),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let groups: Vec<CadGroupStatus> = self
            .core
            .controller
            .borrow()
            .session
            .groups
            .groups()
            .map(CadGroupStatus::from_group)
            .collect();
        let images: Vec<CadImageStatus> = drawing
            .as_deref()
            .map(|database| {
                database
                    .entities()
                    .filter_map(|entity| {
                        let cad_domain::SemanticGeometry::Image {
                            pixels,
                            file,
                            clip,
                            visible,
                            ..
                        } = &entity.geometry
                        else {
                            return None;
                        };
                        Some(CadImageStatus::from_entity(
                            entity.id,
                            &entity.space,
                            file,
                            *pixels,
                            *visible,
                            clip.as_ref(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let entity_count = drawing
            .as_ref()
            .map(|database| database.entity_count() as u64);
        let loading = self.core.loading();
        let can_open = self.core.capabilities.open
            && (!self.core.options.headless || self.core.options.drawing.is_some())
            && !loading;
        // The document-dependent capabilities (NEW/SAVE/PLOT) are a pure
        // projection of the advertised host flags × the real session state: an
        // open/import in flight gates all three, and SAVE/PLOT additionally
        // need an authoritative document and no save/plot target chooser or
        // export still pending (`HostCore::save_pending`, the same slot
        // `request_save`/`request_plot` refuse on).
        let capabilities = document_capabilities(
            self.core.capabilities,
            has_document,
            loading,
            self.core.save_pending(),
        );
        // A frame the caller did not measure falls back to the last real present
        // count the GPU lane recorded; nothing is invented either way.
        let frame_count = if frame_count == 0 {
            self.frames.get()
        } else {
            frame_count
        };
        // The current best snap candidate kind (ADR 0014 A.7) reuses the cached
        // hint resolution the snap overlay consumes, so a per-frame status read
        // never walks the drawing a second time. A resolution failure is never a
        // silent "no candidate": the field stays `null` and the technical reason
        // is folded into `error`, exactly like the other host diagnostics.
        let (snap_kind, snap_error) = match self.core.controller.borrow().snap_hint_kind() {
            Ok(kind) => (kind.map(str::to_string), None),
            Err(error) => (None, Some(cad_error_text(&error))),
        };
        // Whether the host is drawing its own pointer crosshair right now: a 2D
        // view with a pointer position on the surface. The frontend hides the
        // system cursor exactly when this is true, so a 3D view (which draws no
        // crosshair) keeps the system arrow instead of being left pointerless.
        // Read from the host's cursor source (the same derivation the renderer
        // transient uses), never from a guess about the view mode. The very same
        // point is projected as the status-bar world coordinate, so the readout and
        // the drawn crosshair can never disagree.
        let cursor_world = self
            .core
            .cursor_world()
            .map(|point| [point.x, point.y, point.z]);
        let crosshair = cursor_world.is_some();
        // Technical error sources, in priority order: a host status failure
        // (command/open/resource-install), a projected partial-resource report,
        // the last recorded operation/completion failure, the render session's
        // own diagnostic, then a snap-resolution failure.
        let error = self
            .core
            .last_status()
            .as_ref()
            .and_then(status_error)
            .or_else(|| self.resource_error.borrow().clone())
            .or_else(|| self.error.borrow().clone())
            .or_else(|| self.core.render.view_diagnostic())
            .or(snap_error);
        // The projected tool carries the session's typed parameter values, so it is
        // built from the tool snapshot **and** the authoritative session store.
        let tool = {
            let snapshot = self.core.tool_snapshot();
            let controller = self.core.controller.borrow();
            cad_tool(&snapshot, &controller.session)
        };
        let mut status = assemble_status(
            has_document,
            label,
            entity_count,
            loading,
            can_open,
            capabilities,
            error,
            camera,
            backend,
            adapter,
            selection,
            self.import_count.get(),
            frame_count,
            tool,
            self.core.controller.borrow().hover_preview_enabled(),
            self.core.controller.borrow().snap_enabled(),
            self.core.controller.borrow().grid_enabled(),
            self.core.controller.borrow().snap_grid_enabled(),
            self.core.controller.borrow().snap_spacing(),
            self.core.controller.borrow().ortho_enabled(),
            snap_kind,
            crosshair,
            cursor_world,
            // The host's own gesture state: an armed grip drag is real even
            // though `tool` is still `null` (a drag is not a tool).
            self.core.grip_drag_active(),
            layouts,
            active_space,
            theme,
            view,
            navigation,
        );
        // Real live open progress (running only) from the host snapshot, so the
        // chrome can show a determinate/indeterminate bar without inventing data.
        status.import_progress = self
            .core
            .import_snapshot()
            .as_ref()
            .and_then(CadImportProgress::from_snapshot);
        // The authoritative xref rows projected above: always carried, never
        // omitted, and empty for a document that declares none.
        status.references = references;
        status.blocks = blocks;
        status.groups = groups;
        status.images = images;
        // The real bounded command echo (observational only). Cleared by the host
        // on a verified document replacement, so this never mixes drawings.
        status.command_log = self
            .core
            .command_log_entries()
            .iter()
            .map(CadCommandLogEntry::from_host)
            .collect();
        status
    }
}
