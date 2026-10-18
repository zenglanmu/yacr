//! Hover resolution and its observable counters (ADR-0012 §10 S2).
use super::*;
use std::time::Instant;

/// Whether the opt-in pick profile log is enabled.
///
/// The same environment gate the pick path itself uses (`YACR_PROFILE_PICK`,
/// see `cad_app::picking`), read here so the hover wrapper can label its own
/// measurement. Off by default: no always-on metric exists.
fn hover_profile_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("YACR_PROFILE_PICK").is_some())
}

impl HostCore {
    /// Whether a hover query must be suppressed right now.
    ///
    /// The idle entity preview is **on by default**, matching AutoCAD's
    /// `SELECTIONPREVIEW` initial `3`, and `HOVERPREVIEW` turns it off; once off a
    /// plain cursor move runs no pick and leaves no tint. A positive
    /// tool (draw/measure capture or pan), a press/navigation gesture
    /// (pan/orbit/drag), or an active multi-contact navigation owns the pointer;
    /// hover must not compete with it. `Selecting` deliberately does not
    /// suppress: after a click the user must be able to hover the next object.
    /// This is the second line of defence — the session already emits no hover
    /// request for a pressed pointer — so a host that delivers one anyway never
    /// leaves a stale tint behind.
    fn hover_suppressed(&self) -> bool {
        let tool_owns_pointer = matches!(
            &self.controller.borrow().session.tool,
            ToolState::Drawing(_) | ToolState::Measuring(_) | ToolState::Panning
        );
        tool_owns_pointer
            || !self.controller.borrow().session.hover_preview()
            || self.navigation.dragged.get()
            || self.navigation.navigating.get()
            || self.navigation.box_gesture.borrow().is_some()
            || !self.navigation.active.borrow().is_empty()
    }

    /// Clear the hovered set, any pending dwell, and forget the last hover query
    /// position.
    ///
    /// Pure session/overlay state: no database write, no history entry, and no
    /// published shell state change, so nothing is marked dirty here. The
    /// `sync_render` that ends every interaction dispatch republishes the
    /// overlay inputs.
    pub(crate) fn clear_hover(&self) {
        self.hover.hovered.borrow_mut().clear();
        *self.hover.pending.borrow_mut() = None;
        self.hover.last_query.set(None);
        self.hover.last_query_identity.set(None);
    }

    /// The number of hover pick queries this host has actually executed.
    ///
    /// Read-only observable behaviour for the hover throttle contract: an
    /// unchanged cursor position re-runs no pick, so the counter does not move.
    /// It is a counter, never a wall-clock measurement.
    pub fn hover_query_count(&self) -> u64 {
        self.hover.queries.get()
    }

    /// The canvas-local logical cursor position a hover request names.
    ///
    /// A canvas-local point is used as-is and its metrics revision validated
    /// exactly like `SelectAt`'s; a world point is projected through the active
    /// camera for a host that supplies one directly.
    fn hover_logical(&self, point: PointInput) -> CadResult<[f64; 2]> {
        match point {
            PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            } => {
                if metrics_revision != self.metrics_revision.get() {
                    return Err(CadError::InvalidInput(
                        "stale canvas metrics revision".into(),
                    ));
                }
                Ok(logical_position)
            }
            PointInput::World(world) => {
                let c = self.controller.borrow();
                let viewport = c
                    .application
                    .workspace
                    .viewports
                    .get(&c.viewport_id)
                    .ok_or(CadError::Cancelled)?;
                viewport
                    .camera
                    .world_to_screen(world, viewport.logical_size)
                    .ok_or_else(|| CadError::InvalidInput("world point cannot be projected".into()))
            }
        }
    }

    /// Resolve the top-ranked pick under the cursor and publish it as hover.
    ///
    /// Hover reuses the **exact** click-selection path — `pick_at_screen` with
    /// the default `TolerancePolicy` and `BackFacePolicy::Cull`, now through its
    /// layer-visibility-aware entry so a hidden-layer entity is never hovered —
    /// so there is no second tolerance, no second ranking, no second predicate
    /// and no second identity type. Hover is entity-level and single: the one
    /// top-ranked ref the report returns.
    ///
    /// Pickbox aperture (ADR-0012 §10 S2): the highlight is **sticky**. While the
    /// cursor stays within [`TolerancePolicy::pickbox_logical_pixels`] of the
    /// last *query* position — the **same** `PICKBOX`-analogue aperture
    /// [`cad_app::picking::pick_tolerance`] uses, so hover and click can never
    /// disagree — the current hover is kept and no pick runs; only moving
    /// strictly beyond it re-hits. The cached position is also invalidated when
    /// the hover **identity** changes (the drawing scene identity plus the
    /// session layer-override fingerprint), so a stayed-in-place cursor after an
    /// edit/undo or a layer visibility toggle re-resolves instead of keeping a
    /// stale tint; `clear_hover` (pointer down/up, tool start, …) still forgets
    /// it outright.
    ///
    /// When the session's idle preview is disabled (the default) this clears
    /// any tint and pending dwell and returns before taking a pick: a plain
    /// cursor move is not a query.
    ///
    /// Dwell (an explicit product extension, not AutoCAD): when the session's
    /// `hover_dwell_ms` is non-zero, a freshly resolved ref that differs from the
    /// applied one is not tinted immediately — it is recorded as a
    /// [`PendingHover`] and applied by [`HostCore::apply_due_hover`] once it has
    /// been stable for the dwell. `0` restores the immediate highlight.
    pub(super) fn resolve_hover(&self, point: PointInput) -> CadResult<()> {
        if self.hover_suppressed() {
            self.clear_hover();
            return Ok(());
        }
        let logical = self.hover_logical(point)?;
        // The one tolerance policy the pick itself uses. Its
        // `pickbox_logical_pixels` is also the sticky-re-query aperture, so the
        // throttle and the pick can never disagree about their radius.
        let policy = cad_domain::TolerancePolicy::default();
        // The identity a resolved hover depends on (ADR-0012 §5): the drawing's
        // scene identity (database id + content revision + structural counts +
        // memoized model bounds) **and** the session layer-override fingerprint
        // (spec §4.8: a pure visibility change rebuilds the scene). An edit or
        // undo changes the first; a layer show/hide changes the second; a camera
        // move changes neither.
        let identity = {
            let c = self.controller.borrow();
            c.drawing().map(|drawing| {
                (
                    drawing.scene_identity(),
                    c.session.layer_overrides.fingerprint(),
                )
            })
        };
        let same_identity = identity.is_some() && self.hover.last_query_identity.get() == identity;
        if let Some(last) = self.hover.last_query.get() {
            if same_identity {
                let dx = logical[0] - last[0];
                let dy = logical[1] - last[1];
                let aperture = policy.pickbox_logical_pixels;
                // Strictly more than the aperture re-hits; inside it the
                // highlight persists and no pick runs.
                if dx * dx + dy * dy <= aperture * aperture {
                    return Ok(());
                }
            }
        }
        let profile = hover_profile_enabled();
        let started = profile.then(Instant::now);
        self.metrics()?;
        let picked = {
            let c = self.controller.borrow();
            let viewport = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?;
            let drawing = c.drawing().ok_or(CadError::Cancelled)?;
            let report = cad_app::picking::pick_at_screen_with_visibility(
                &drawing,
                c.document_id,
                &viewport.camera,
                logical,
                viewport.logical_size,
                &policy,
                cad_app::BackFacePolicy::Cull,
                &c.session.layer_overrides,
            );
            report.map(|report| report.hit.map(|hit| hit.source))
        };
        let picked = match picked {
            Ok(picked) => picked,
            Err(error) => {
                // A hover that cannot be resolved is an explicit refusal, never
                // a silent "still hovering the previous entity".
                self.clear_hover();
                return Err(error);
            }
        };
        if let Some(started) = started {
            // Opt-in (`YACR_PROFILE_PICK`, the same gate the pick path itself
            // uses) and measured, never estimated. A local/synthetic number: it
            // is not a performance conclusion.
            eprintln!(
                "YACR_PROFILE_PICK hover elapsed_ms={:.3} picked={}",
                started.elapsed().as_secs_f64() * 1000.0,
                picked.is_some()
            );
        }
        self.hover.last_query.set(Some(logical));
        self.hover.last_query_identity.set(identity);
        self.hover
            .queries
            .set(self.hover.queries.get().saturating_add(1));
        let dwell_ms = self.controller.borrow().session.hover_dwell_ms();
        self.apply_resolved_hover(picked, dwell_ms);
        Ok(())
    }

    /// Apply a freshly resolved hover, honouring the session dwell.
    ///
    /// * `dwell_ms == 0`: immediate (the AutoCAD-immediate behaviour).
    /// * a ref equal to the applied one: no change, and any pending dwell is
    ///   dropped (the highlight is already stable).
    /// * `None`: the applied tint is cleared immediately — leaving geometry must
    ///   never leave a stale tint for a whole dwell; the pending dwell is dropped.
    /// * a different `Some(ref)`: recorded as pending with a fresh deadline. A
    ///   move that re-resolves the **same** pending ref keeps its deadline (so the
    ///   dwell can elapse); a move that yields a different ref restarts it (a
    ///   fast sweep paints nothing).
    fn apply_resolved_hover(&self, picked: Option<SelectionRef>, dwell_ms: u64) {
        if dwell_ms == 0 {
            *self.hover.pending.borrow_mut() = None;
            self.set_applied_hover(picked);
            return;
        }
        let applied_single = {
            let applied = self.hover.hovered.borrow();
            (applied.len() == 1).then(|| applied.refs()[0].clone())
        };
        match picked {
            None => {
                *self.hover.pending.borrow_mut() = None;
                self.set_applied_hover(None);
            }
            Some(reference) => {
                if applied_single.as_ref() == Some(&reference) {
                    // The tint already matches: stable, nothing to defer.
                    *self.hover.pending.borrow_mut() = None;
                    return;
                }
                let mut pending = self.hover.pending.borrow_mut();
                let already_pending = pending
                    .as_ref()
                    .is_some_and(|existing| existing.reference == reference);
                if !already_pending {
                    *pending = Some(PendingHover {
                        reference,
                        resolved_at: (self.hover_now)(),
                    });
                }
            }
        }
    }

    /// Replace the applied (tinted) hover set with `picked`, if it differs.
    fn set_applied_hover(&self, picked: Option<SelectionRef>) {
        let mut hovered = self.hover.hovered.borrow_mut();
        let changed = match &picked {
            // Hover is single: an equal one-ref set is not a change.
            Some(reference) => !(hovered.len() == 1 && hovered.contains(reference)),
            None => !hovered.is_empty(),
        };
        if changed {
            match picked {
                Some(reference) => hovered.replace([reference]),
                None => hovered.clear(),
            }
        }
    }

    /// Apply a pending hover whose dwell has elapsed, if any.
    ///
    /// Called once per frame from [`HostCore::tick`] — this is the only timing
    /// hook; there is no timer thread. Returns `true` when the applied hover set
    /// changed, so the caller knows the render overlay inputs need republishing.
    /// A call with nothing due touches no state, so a stable tick schedules no
    /// render revision churn.
    pub(crate) fn apply_due_hover(&self) -> bool {
        let due = {
            let pending = self.hover.pending.borrow();
            pending
                .as_ref()
                .map(|pending| (pending.reference.clone(), pending.resolved_at))
        };
        let Some((reference, resolved_at)) = due else {
            return false;
        };
        let dwell_ms = self.controller.borrow().session.hover_dwell_ms();
        if dwell_ms > 0
            && (self.hover_now)()
                .saturating_duration_since(resolved_at)
                .as_millis()
                < u128::from(dwell_ms)
        {
            return false;
        }
        *self.hover.pending.borrow_mut() = None;
        let already_applied = {
            let hovered = self.hover.hovered.borrow();
            hovered.len() == 1 && hovered.contains(&reference)
        };
        if already_applied {
            return false;
        }
        self.set_applied_hover(Some(reference));
        true
    }
}
