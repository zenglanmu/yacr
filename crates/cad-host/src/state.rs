//! Fallible, UI-independent state derivation and dirty update publication.
use super::*;

impl HostCore {
    pub(crate) fn sync_render(&self) -> CadResult<()> {
        // Install the configured pickbox/aperture policy before anything on the
        // snap path reads it (the snap-hint resolution below does). The same
        // policy feeds the cursor overlay through `pickbox_logical_pixels`, so the
        // drawn box, the click predicate and the snap aperture all come from one
        // effective config.
        self.seed_interaction_tolerance();
        let (
            drawing,
            view,
            layers,
            selection,
            measurement,
            preview,
            snap_hints,
            cursor,
            view_scale,
        ) = {
            let c = self.controller.borrow();
            let viewport = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?;
            let space = match &c.session.active_space {
                SpaceId::Model => cad_representation::SpaceSelection::Model,
                SpaceId::Paper(id) => cad_representation::SpaceSelection::Paper(*id),
                SpaceId::Block(_) => {
                    return Err(CadError::InvalidInput(
                        "block space cannot be displayed directly".into(),
                    ))
                }
            };
            // The frame's background: the active space's configured colour, as
            // the renderer's linear channels. Model and paper really are
            // distinguishable here because the clear is per frame and this
            // snapshot names the frame's space.
            let clear_color = self
                .viewer_config
                .borrow()
                .effective()
                .view
                .background
                .resolve(matches!(
                    space,
                    cad_representation::SpaceSelection::Paper(_)
                ))
                .to_linear_rgba();
            let view = ViewSnapshot {
                document: c.document_id,
                camera: viewport.camera,
                // The session view mode is authoritative: a legitimate 3D view
                // with an orthographic camera must stay `ThreeD`, so this never
                // guesses from the camera's projection variant.
                mode: viewport.view_mode.kind(),
                space,
                canvas_size: viewport.logical_size,
                clear_color,
            };
            // The pointer-cursor overlay input: the cursor's world point on the
            // work plane plus the view scale (world units per logical pixel),
            // both only for a 2D view with a cursor present. A 3D view has no
            // planar cursor and no view rectangle, and an absent cursor is an
            // explicit empty overlay rather than a fabricated one.
            let two_d = viewport.view_mode.kind() == cad_app::ProjectionKind::TwoD;
            let (cursor, view_scale) = match (two_d, self.cursor_logical.get()) {
                (true, Some(logical)) => {
                    match viewport.screen_to_world(logical, viewport.logical_size) {
                        Some(world) => (Some(world), Some(viewport.world_per_px())),
                        None => (None, None),
                    }
                }
                _ => (None, None),
            };
            (
                c.drawing(),
                view,
                c.session.layer_overrides.clone(),
                c.selection().clone(),
                c.measurement_preview(),
                c.session.draw_preview(),
                c.snap_hints_near_cursor()?,
                cursor,
                view_scale,
            )
        };
        self.render.sync_drawing(drawing);
        self.render.apply_view_snapshot(view)?;
        self.render.set_layers(layers);
        // Build the overlay inputs with the *final* viewport bounds in one shot.
        // Passing `None` here and then calling `set_viewport_bounds` mutated the
        // overlay input twice per sync, so every idle tick bumped the render
        // input/scene revision even with no state change.
        //
        // `hovered` carries the host's transient hover state (ADR-0012 §10 S2),
        // which the `MoveCursor` handler resolves through the click-selection
        // pick path and clears on a miss/suppression. `hidden` stays the
        // hardcoded empty set: its producer is gated by the id pass honouring
        // `FLAG_HIDDEN` first (backlog G2/G5), and wiring it early would leave
        // hidden entities still pickable and occluding instead of openly
        // unsupported (rule 2).
        //
        // Capture-candidate tint (TRIM/EXTEND). A drawing capture owns the
        // pointer, so idle hover is suppressed for its whole duration and the
        // entity under the cursor is published on `DrawPreview::candidate`
        // instead. Feed the capture's picks and candidate through the
        // **existing** `hovered` input / `FLAG_HOVERED` tint rather than adding
        // a second flag, overlay input or colour: `hovered` already means "the
        // entity under the pointer", the two producers can never compete (idle
        // hover is cleared for the capture), and a distinct candidate colour
        // would be a renderer change, not a host one.
        //
        // The tint is the committed picks **and** the pending candidate as one
        // set, so the entity a step already committed stays visible while the
        // next one is chosen — the way AutoCAD shows the cutting edge(s) as the
        // objects are picked. It is empty when there is neither. Any other
        // active capture forces the set empty, so an idle hover can never become
        // a second producer.
        //
        // The set is filtered to refs that still resolve against the
        // authoritative drawing (see [`HostCore::capture_pick_tint`]): the
        // command line is not capture-modal, so an edit that ran mid-capture can
        // leave a pick pointing at a vanished entity, and feeding that to the
        // flags path would turn it into a `highlight.unresolved` diagnostic.
        let hovered = match &preview {
            Some(preview) if preview.kind.needs_picked_entities() => {
                self.capture_pick_tint(preview)
            }
            Some(_) => cad_app::SelectionSet::new(),
            None => self.hover.hovered.borrow().clone(),
        };
        let (overlay_visibility, crosshair_percent, pickbox_logical_pixels) = {
            let store = self.viewer_config.borrow();
            let view = &store.effective().view;
            (
                OverlayVisibility::from(view.overlays.clone()),
                view.crosshair.size_percent,
                // The same policy the pick/hover paths use, so the drawn box is
                // exactly the click radius (see `cursor_overlay`).
                view.tolerance_policy().pickbox_logical_pixels,
            )
        };
        let overlays = OverlayInputs {
            selection,
            hovered,
            hidden: cad_app::SelectionSet::new(),
            measurement,
            draw_preview: preview,
            // The live box-selection rubber band (ADR 0014 B.9): a transient
            // gesture input, never a database write. `None` unless a 2D primary
            // drag is in progress.
            selection_box: self.box_selection_preview(),
            snap_hints_input: snap_hints,
            visibility: overlay_visibility,
            viewport_bounds: self.canvas_bounds(),
            // Visible POINT markers are derived by the scene assembly from the
            // drawing being displayed, so the host feeds none itself.
            point_markers: Vec::new(),
            // Screen-space sizing for the transient overlays (snap markers, the
            // cursor crosshair/pickbox): world units per logical pixel of the 2D
            // view, and the cursor's world point.
            view_scale,
            cursor,
            crosshair_percent,
            pickbox_logical_pixels,
        };
        self.render.set_overlays(overlays);
        let _ = self.render.request_preparation()?;
        Ok(())
    }

    /// The `FLAG_HOVERED` set a TRIM/EXTEND capture publishes: its committed
    /// picks and its pending candidate as **one** set, in pick-then-candidate
    /// order, empty when there is neither.
    ///
    /// Only refs that still resolve against the authoritative drawing are
    /// included. The command line is not capture-modal, so a command can replace
    /// the document in the middle of a capture (an `ERASE`/`UNDO` typed at the
    /// prompt) and leave a pick or candidate pointing at a vanished entity.
    /// Feeding such a ref to the flags path would produce a
    /// `highlight.unresolved` "… is not present in the document" error — the
    /// exact P2 error class this tint must not reintroduce. A non-resolving ref
    /// is simply not tinted (never fabricated, never forced through); the
    /// capture itself stays alive and a `CONFIRM` over a vanished pick is
    /// refused by the command layer.
    ///
    /// Presentation only: the capture's own `DrawPreview::picks` remain the
    /// commit source and are never mutated here.
    fn capture_pick_tint(&self, preview: &cad_app::DrawPreview) -> cad_app::SelectionSet {
        let c = self.controller.borrow();
        // No authoritative drawing means nothing can be verified to resolve, so
        // tint nothing rather than fabricate.
        let Some(drawing) = c.drawing() else {
            return cad_app::SelectionSet::new();
        };
        let document = c.document_id;
        let refs = preview
            .picks
            .iter()
            .chain(preview.candidate.iter())
            .filter(|reference| {
                cad_app::picking::resolve_pick_transform(&drawing, document, reference).is_some()
            })
            .cloned();
        cad_app::SelectionSet::from_refs(refs)
    }

    /// The world-space rectangle the active 2D plan viewport covers.
    ///
    /// Mirrors the renderer's `viewport_world_bounds` contract exactly: an
    /// **axis-ordered** `(min, max)` pair, and no extent at all for a 3D view
    /// (a 3D view has no planar 2D rectangle, so it must not fabricate one).
    ///
    /// Screen Y grows downward, so the corner projections are normalized per
    /// axis over all four corners. Passing the top-left/bottom-right pair
    /// straight through as `(a, b)` left `min.y > max.y`, which collapsed every
    /// horizontal overlay/grid line (the vertical ones happened to survive).
    fn canvas_bounds(&self) -> Option<(Point3, Point3)> {
        let c = self.controller.borrow();
        let viewport = c.application.workspace.viewports.get(&c.viewport_id)?;
        if viewport.view_mode.kind() != cad_app::ProjectionKind::TwoD {
            return None;
        }
        let [width, height] = viewport.logical_size;
        let corners = [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]];
        let mut min = Point3 {
            x: f64::INFINITY,
            y: f64::INFINITY,
            z: f64::INFINITY,
        };
        let mut max = Point3 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
            z: f64::NEG_INFINITY,
        };
        for corner in corners {
            let point = viewport.screen_to_world(corner, viewport.logical_size)?;
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
        [min.x, min.y, min.z, max.x, max.y, max.z]
            .iter()
            .all(|value| value.is_finite())
            .then_some((min, max))
    }

    pub fn sync_view(&self) -> CadResult<()> {
        self.sync_render()
    }

    /// The render session's raw, *actual* backend state.
    ///
    /// Derived from the renderer's real outcome/label, never from the requested
    /// preference, so a forced WebGL2 that actually ran on WebGPU still reports
    /// WebGPU. Used to publish a backend-only delta without a full state query.
    pub(crate) fn backend_state(&self) -> RenderBackendState {
        RenderBackendState {
            outcome: self.render.backend_outcome(),
            api: self.render.backend_label(),
        }
    }

    pub(crate) fn navigation_state(&self) -> CadResult<NavigationState> {
        let c = self.controller.borrow();
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .ok_or(CadError::Cancelled)?;
        let hints = c.snap_hints_near_cursor()?;
        Ok(NavigationState {
            viewport: c.viewport_id,
            camera: viewport.camera,
            // Follow the session view mode, not the camera projection, so a 3D
            // orthographic view reports `ThreeD` consistently with the snapshot.
            mode: viewport.view_mode.kind(),
            space: c.session.active_space.clone(),
            snap_hints: hints
                .into_iter()
                .map(|hint| SnapHint {
                    // The snap engine's own stable, locale-independent key
                    // (never a `Debug`-derived spelling).
                    kind: hint.kind.as_str().to_string(),
                    point: hint.point,
                    space: hint.space,
                    source: hint.source,
                    secondary: hint.secondary,
                    precision: hint.precision,
                    logical_pixel_distance: hint.logical_pixel_distance,
                })
                .collect(),
        })
    }

    pub(crate) fn full_state(&self) -> CadResult<HostState> {
        let loading = self.loading();
        let import_progress = self.import_snapshot();
        let c = self.controller.borrow();
        let drawing = c.drawing().ok_or(CadError::Cancelled)?;
        let layers = c.layer_rows()?;
        let selection_properties = c.selection_properties()?;
        let import = c.last_import_report.as_ref().map(|report| ImportState {
            identity: report.identity.clone(),
            capabilities: report
                .capabilities
                .iter()
                .map(|item| RawEntityCapability {
                    type_key: item.type_key.clone(),
                    read: item.read,
                    semantic: item.semantic,
                    render: item.render,
                    pick: item.pick,
                    measure: item.measure,
                })
                .collect(),
            completeness: report.completeness.clone(),
            diagnostics: report.diagnostics.clone(),
            parse_ms: report.parse_ms,
        });
        let mut diagnostics: Vec<RawDiagnostic> = c
            .last_import_report
            .as_ref()
            .into_iter()
            .flat_map(|report| report.diagnostics.iter())
            .map(|diagnostic| RawDiagnostic {
                object: diagnostic.object,
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                severity: None,
                completeness: None,
                parameters: Vec::<RawDiagnosticParameter>::new(),
            })
            .collect();
        // Raster images that did not resolve (missing file) or could not decode
        // are document-level `resource.missing` diagnostics, never a silent
        // drop. Only the stable code and the raw key travel here — the UI drawer
        // localizes them; the host carries no prose.
        if let Some(report) = self.last_image_report.borrow().as_ref() {
            for key in report.unresolved.iter().chain(report.failed.iter()) {
                diagnostics.push(RawDiagnostic {
                    object: None,
                    code: cad_diagnostics::model::codes::RESOURCE_MISSING.to_string(),
                    // No source message exists for a synthesized resource
                    // diagnostic; the drawer resolves `code` through its catalog.
                    message: String::new(),
                    // Mirrors the pre-decoupling `DiagnosticReason::missing`
                    // semantics exactly: an error-severity, `Missing` reason, so
                    // the UI drawer folds it into its completeness summary.
                    severity: Some(cad_engine::protocol::DiagnosticSeverity::Error),
                    completeness: Some(cad_domain::Completeness::Missing(Vec::new())),
                    parameters: vec![RawDiagnosticParameter::Key(key.clone())],
                });
            }
        }
        let document = c
            .application
            .workspace
            .documents
            .get(&c.document_id)
            .ok_or(CadError::Cancelled)?;
        Ok(HostState {
            capabilities: HostCapabilitiesState {
                open: self.capabilities.open
                    && (!self.options.headless || self.options.drawing.is_some())
                    && !loading,
                new: self.capabilities.new && !loading,
                save: self.capabilities.save && !loading,
                plot: self.capabilities.plot && !loading,
            },
            document_name: c.document_name_hint.clone(),
            mode: c.mode(),
            history: c.history_availability(),
            layer_rows: layers,
            selection_properties,
            layouts: cad_app::render_scene::layout_descriptors(&drawing),
            active_space: c.session.active_space.clone(),
            draw_preview: c.session.draw_preview(),
            // `Some` only while a drawing tool is actively capturing; idle must
            // not fall back to a synthetic `Line`.
            selected_draw_kind: match &c.session.tool {
                ToolState::Drawing(capture) => Some(capture.tool.kind()),
                _ => None,
            },
            measurement_preview: c.measurement_preview(),
            selected_measurement_kind: c.session.selected_measurement_kind(),
            // The document's real unit context (unknown units stay "drawing
            // units"); only a missing document would be `None`, and `full_state`
            // already refuses without one.
            units: Some(document.units.clone()),
            import_progress,
            diagnostics,
            import,
            resources: ResourceState {
                fonts: None,
                images: self.last_image_report.borrow().as_ref().map(|r| {
                    cad_engine::protocol::ImageResourceState {
                        requested: r.requested.clone(),
                        loaded: r.loaded.clone(),
                        unresolved: r.unresolved.clone(),
                        failed: r.failed.clone(),
                        bytes: r.bytes,
                    }
                }),
                proxy: None,
                references: Some(document.resource_keys.clone()),
                images_modeled: true,
            },
        })
    }

    pub fn push(&self) -> CadResult<()> {
        self.full_dirty.set(true);
        self.take_updates().map(|_| ())
    }

    pub fn push_navigation(&self) -> CadResult<()> {
        self.navigation_dirty.set(true);
        self.take_updates().map(|_| ())
    }
}
