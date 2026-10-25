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
//! * **no undo entry is recorded**: layer edits are not undoable because
//!   `cad-history` stores entity patches only. A layer patch kind is a separate
//!   lane; advertising undo here would restore nothing.
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

use super::*;
use crate::layer_property::LayerPropertyValue;

impl Application {
    /// Dispatch the layer-table commands.
    pub(crate) fn layer_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
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

        let outcome = match &command.payload {
            CommandPayload::CreateLayer(name) => {
                let (id, changeset) = candidate.create_layer(name, transaction, "create layer")?;
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
                let changeset = candidate.rename_layer(id, name, transaction, "rename layer")?;
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
                let changeset = candidate.delete_layer(id, transaction, "delete layer")?;

                // Post-commit session hygiene: neither the temporary override nor
                // the active layer may keep a reference to the removed row.
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
                            "active layer '{deleted}' was deleted; the active layer is now '{}'",
                            cad_db::DEFAULT_LAYER_NAME
                        ),
                    });
                }
                CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(Arc::new(changeset)),
                    diagnostics,
                    measurement: None,
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
                // claiming a change.
                let changed = changeset.before != changeset.after;
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
                }
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "not a layer table command".to_string(),
                ))
            }
        };

        // Publish the already-validated candidate. No history record: layer edits
        // are not undoable (see the module docs).
        self.workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
            .drawing = Arc::new(candidate);
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
fn resolve_layer(database: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
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
