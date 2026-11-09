//! Layer table commands (`CREATELAYER` / `RENAMELAYER` / `DELETELAYER` /
//! `SETLAYERPROPERTY`).
//!
//! These are the only commands that change the drawing's layer **table** — as
//! opposed to the session-scoped visibility overrides in [`crate::layers`] and
//! the session-only active layer of [`CommandId::SetActiveLayer`]. The real work
//! happens in the database's validated write path
//! ([`cad_db::DrawingDatabase::create_layer`] / `rename_layer` / `delete_layer` /
//! `set_layer_attributes`); this module only resolves the panel-supplied layer
//! **name**, parses the attribute value with the one layer grammar
//! ([`crate::layer_property`]), applies the candidate database, keeps the session
//! consistent and reports what happened.
//!
//! Contract:
//!
//! * every refusal happens **before** the candidate is published, so the open
//!   document, its revision and the session are untouched;
//! * the attribute write parses and resolves its value (`layer_property`) and
//!   applies it to a **cloned** attribute set, so a refused key/value leaves the
//!   stored attributes byte-identical; `off`/`frozen` keep
//!   `Layer::visible == !(off || frozen)` because the database recomputes it in
//!   the same mutation (never a second, separate write);
//! * a successful write replaces the document's `Arc<DrawingDatabase>` and
//!   publishes the database's `ChangeSet` (new revision, no entity changes), the
//!   documented staleness signal every projection binds to;
//! * **every real write records exactly one layer-table undo step**: the
//!   before/after [`cad_db::LayerRow`] pair goes through
//!   [`Application::publish_layer_record`], so create (`before: None`), rename,
//!   delete (`after: None`) and attribute change each undo and redo in one
//!   revision. An authoritative no-op (rename to the stored name, writing an
//!   identical attribute set) records nothing, exactly like before.
//!   `active_layer` and the temporary visibility overrides are **session** state
//!   and are deliberately not history: undoing a delete restores the layer row,
//!   but not the session's choice of active layer (deleting the active layer also
//!   falls back to `0`; the fallback is reported and not undone);
//! * the temporary visibility override set is pruned when its layer is deleted,
//!   so no override outlives the row it referred to;
//! * **a write never hides the session's current layer**
//!   ([`ensure_current_layer_stays_visible`]): `off=true` / `frozen=true` on the
//!   active layer is refused (`layer.current_off` / `layer.current_frozen`)
//!   before any candidate attribute is touched. The shared create funnel does not
//!   look at visibility, so allowing that write would store geometry the user can
//!   neither see nor pick — the same argument that makes
//!   [`Application::set_active_layer`] refuse an invisible layer as current
//!   (`app_drawing::drawing_create`). Clearing the flags (`false`), and every
//!   other key including `locked`, stay allowed on the current layer, and a
//!   non-current layer can still be turned off or frozen.

use super::layer_record::layer_row;
use super::*;
use crate::layer_property::LayerPropertyValue;

impl Application {
    /// Dispatch the layer-table commands.
    ///
    /// The ordinary dispatch records exactly one layer patch per command; the
    /// no-history variant ([`Self::layer_command_without_history`]) is the legacy
    /// `PURGE` reuse and publishes without a history entry.
    pub(crate) fn layer_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.layer_command_recording(session, command, true)
    }

    /// Delete a layer row through the same validated path as `DELETELAYER` but
    /// publish **without** an undo record.
    ///
    /// Only `PURGE` uses this: it keeps its documented non-undoable behaviour,
    /// while the ordinary `DELETELAYER` path records one step.
    pub(crate) fn layer_command_without_history(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.layer_command_recording(session, command, false)
    }

    fn layer_command_recording(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        record_history: bool,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let mut candidate = drawing.as_ref().clone();
        let transaction = next_transaction_id();
        // One record per command: label plus the exact before/after row(s).
        let mut pending: Option<(&'static str, Vec<cad_db::LayerPatch>)> = None;

        let outcome = match &command.payload {
            CommandPayload::CreateLayer(name) => {
                let (id, changeset) = candidate.create_layer(name, transaction, "create layer")?;
                pending = Some((
                    "create layer",
                    vec![cad_history::layer_patch(
                        id,
                        None,
                        layer_row(&candidate, id),
                    )],
                ));
                let stored = candidate
                    .layer(id)
                    .map(|layer| layer.name.clone())
                    .unwrap_or_else(|| name.clone());
                CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(changeset)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "drawing.layer_created".into(),
                        message: format!("layer '{stored}' created (id {})", id.0),
                    }],
                    measurement: None,
                    point_report: None,
                }
            }
            CommandPayload::RenameLayer { layer, name } => {
                let id = resolve_layer(drawing.as_ref(), layer)?;
                let previous = drawing
                    .layer(id)
                    .map(|row| row.name.clone())
                    .ok_or_else(|| {
                        CadError::Invariant("resolved layer vanished before the rename".into())
                    })?;
                let before = layer_row(drawing.as_ref(), id);
                let changeset = candidate.rename_layer(id, name, transaction, "rename layer")?;
                // Renaming to the stored name is an authoritative no-op: no
                // revision move, so no undo entry either.
                if changeset.before != changeset.after {
                    pending = Some((
                        "rename layer",
                        vec![cad_history::layer_patch(
                            id,
                            before,
                            layer_row(&candidate, id),
                        )],
                    ));
                }
                let stored = candidate
                    .layer(id)
                    .map(|row| row.name.clone())
                    .unwrap_or_else(|| name.clone());
                CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(changeset)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "drawing.layer_renamed".into(),
                        message: format!("layer '{previous}' renamed to '{stored}'"),
                    }],
                    measurement: None,
                    point_report: None,
                }
            }
            CommandPayload::DeleteLayer { layer } => {
                let id = resolve_layer(drawing.as_ref(), layer)?;
                let deleted = drawing
                    .layer(id)
                    .map(|row| row.name.clone())
                    .ok_or_else(|| {
                        CadError::Invariant("resolved layer vanished before the delete".into())
                    })?;
                // The session must never keep pointing at a deleted row. Decide
                // the fallback **before** mutating: if this delete removes the
                // active layer and the drawing has no `0` row to fall back to,
                // the honest answer is a refusal, not a dangling active id.
                let fallback = if session.active_layer == id {
                    Some(
                        drawing
                            .layers()
                            .find(|row| row.name == cad_db::DEFAULT_LAYER_NAME)
                            .map(|row| row.id)
                            .ok_or_else(|| {
                                CadError::InvalidInput(format!(
                                    "layer.default_missing: cannot delete the active layer '{deleted}' \
                                     because this drawing has no '{}' layer to fall back to",
                                    cad_db::DEFAULT_LAYER_NAME
                                ))
                            })?,
                    )
                } else {
                    None
                };
                let before = layer_row(drawing.as_ref(), id);
                let changeset = candidate.delete_layer(id, transaction, "delete layer")?;
                pending = Some((
                    "delete layer",
                    vec![cad_history::layer_patch(id, before, None)],
                ));

                // Post-commit session hygiene: neither the temporary override nor
                // the active layer may keep a reference to the removed row.
                //
                // Deliberately **not** a `LAYERP` step (`crate::layer_history`
                // records layer-*view* mutations: toggle/batch/isolate/layoff/
                // restore). Pushing here would let `LAYERP` restore an override on
                // a layer id that no longer exists. Undo restores the deleted row
                // (through the layer patch below) but not these session choices.
                session.layer_overrides.remove(id);
                let mut diagnostics = vec![Diagnostic {
                    object: None,
                    code: "drawing.layer_deleted".into(),
                    message: format!("layer '{deleted}' (id {}) deleted", id.0),
                }];
                if let Some(fallback) = fallback {
                    session.active_layer = fallback;
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "drawing.active_layer_fallback".into(),
                        message: format!(
                            "active layer '{deleted}' was deleted; the active layer is now '{}' \
                             (this session choice is not part of the layer-table undo step)",
                            cad_db::DEFAULT_LAYER_NAME
                        ),
                    });
                }
                CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(changeset)),
                    diagnostics,
                    measurement: None,
                    point_report: None,
                }
            }
            CommandPayload::SetLayerProperty { layer, key, value } => {
                let id = resolve_layer(drawing.as_ref(), layer)?;
                let name = drawing
                    .layer(id)
                    .map(|row| row.name.clone())
                    .ok_or_else(|| {
                        CadError::Invariant("resolved layer vanished before the write".into())
                    })?;
                // Parse **everything** first: the value grammar (which needs the
                // drawing's own linetype table) decides before any candidate
                // attribute is touched.
                let parsed = crate::layer_property::parse(drawing.as_ref(), key, value)?;
                // The visibility rule of the **session's** current layer sits
                // here, before the candidate is touched: the command layer is the
                // only place that knows which layer is current, and `cad-db`
                // keeps owning the invariant (`visible == !(off || frozen)`) with
                // no second copy of this policy.
                ensure_current_layer_stays_visible(session, id, &name, &parsed)?;
                let before = layer_row(drawing.as_ref(), id);
                let mut attributes = drawing.layer_attributes(id);
                crate::layer_property::apply(&mut attributes, parsed);
                // The database recomputes `visible` from the new `off`/`frozen`
                // pair inside this one call, so the invariant is never published
                // half-applied and there is no second write to forget.
                let changeset = candidate.set_layer_attributes(
                    id,
                    attributes.clone(),
                    transaction,
                    "set layer property",
                )?;
                // A write of the stored value is an authoritative no-op: the
                // revision did not move, so the report says so instead of
                // claiming a change, and no undo entry is recorded.
                let changed = changeset.before != changeset.after;
                if changed {
                    pending = Some((
                        "set layer property",
                        vec![cad_history::layer_patch(
                            id,
                            before,
                            layer_row(&candidate, id),
                        )],
                    ));
                }
                CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(changeset)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: if changed {
                            "drawing.layer_property_changed".into()
                        } else {
                            "drawing.layer_property_unchanged".into()
                        },
                        message: format!(
                            "layer '{name}' property '{key}' written as '{value}'{}",
                            if changed {
                                ""
                            } else {
                                " (already stored, no change)"
                            }
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                }
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "not a layer table command".to_string(),
                ))
            }
        };

        // Publish the already-validated candidate with the one record this
        // command produced. A branch that changed nothing records nothing (the
        // record type rejects an empty patch list, and a no-op must not invent a
        // step). `PURGE`'s reuse publishes without history.
        match pending {
            Some((label, patches)) if record_history => {
                self.publish_layer_record(document_id, candidate, transaction, label, patches)?;
            }
            _ => self.publish_layer_table(document_id, candidate)?,
        }
        Ok(outcome)
    }
}

/// Refuse a layer-table write that would hide the session's **current** layer.
///
/// The one rule this build has for "the current layer must stay visible", and it
/// lives at the command layer because only the session knows which layer is
/// current: `cad-db` keeps owning `Layer::visible == !(off || frozen)` and its
/// API is unchanged (`set_layer_attributes` still writes whatever a caller
/// validated).
///
/// Refused only for `off = true` (`layer.current_off`) and `frozen = true`
/// (`layer.current_frozen`) on the active layer; both are deliberate refusals
/// rather than AutoCAD emulation, for the reason the module docs give: the shared
/// create funnel does not look at visibility, so a hidden current layer would
/// accept geometry that is then invisible and unpickable.
///
/// Everything else stays allowed **including `locked`** — locking is a write
/// permission, not visibility, so it does not hide anything — and clearing a
/// flag (`off = false`, `frozen = false`) is always accepted because it can only
/// make the layer more visible. A non-current layer may still be turned off or
/// frozen: the panel's chips keep working for every other row.
///
/// The refusal changes nothing: it runs on the parsed value, before the candidate
/// attribute set exists, so the stored attributes, `Layer::visible`, the
/// revision, the session and the history are all untouched. `name` is the row's
/// own stored spelling, so the message names the layer the user sees.
fn ensure_current_layer_stays_visible(
    session: &SessionState,
    id: LayerId,
    name: &str,
    parsed: &LayerPropertyValue,
) -> CadResult<()> {
    if session.active_layer != id {
        return Ok(());
    }
    let (code, action) = match parsed {
        LayerPropertyValue::Off(true) => ("layer.current_off", "turning it off"),
        LayerPropertyValue::Frozen(true) => ("layer.current_frozen", "freezing it"),
        _ => return Ok(()),
    };
    Err(CadError::InvalidInput(format!(
        "{code}: layer '{name}' ({}) is the current layer; {action} would make geometry created \
         on it invisible, so the write is refused; switch the current layer first",
        id.0
    )))
}

/// Resolve a panel-supplied layer name against the drawing's own layer table.
///
/// Exact name match wins; only when there is none is a **unique**
/// case-insensitive match accepted (AutoCAD symbol-table names are
/// case-insensitive). This mirrors the session's resolver
/// (`cad_ui_core::session::layers::CadSession::resolve_layer`) including its
/// stable reason codes: the panel shows a name match and the command resolves it
/// with the same rule, so the two can never disagree. Unknown and ambiguous names
/// are distinct explicit refusals and neither changes any state.
pub(super) fn resolve_layer(database: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
    let mut exact = database.layers().filter(|layer| layer.name == name);
    if let Some(layer) = exact.next() {
        if exact.next().is_none() {
            return Ok(layer.id);
        }
        return Err(CadError::InvalidInput(format!(
            "layer.ambiguous_name: more than one drawing layer is named '{name}'"
        )));
    }
    let needle = name.to_lowercase();
    let matches: Vec<LayerId> = database
        .layers()
        .filter(|layer| layer.name.to_lowercase() == needle)
        .map(|layer| layer.id)
        .collect();
    match matches.as_slice() {
        [only] => Ok(*only),
        [] => Err(CadError::InvalidInput(format!(
            "layer.unknown: no drawing layer named '{name}'"
        ))),
        _ => Err(CadError::InvalidInput(format!(
            "layer.ambiguous_name: {} drawing layers match '{name}' case-insensitively",
            matches.len()
        ))),
    }
}
