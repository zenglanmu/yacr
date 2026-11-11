//! The typed `-LAYER <option> …` write forms: AutoCAD's command-line layer
//! editor (`-LAYER`, not the `LAYER` dialog).
//!
//! One option per typed line, all through the same validated layer-table path
//! (`cad-db`'s `create_layer` / `rename_layer` / `delete_layer` /
//! `set_layer_attributes`) the panel commands use. The contracts:
//!
//! * **atomic per line**: every name is resolved and every value is parsed
//!   against the authoritative drawing before the *candidate* database clone is
//!   published, so a refusal at any point leaves the drawing, its revision, the
//!   session and the history exactly as they were — the whole line is refused,
//!   never a partially applied prefix;
//! * **one undo step per real line**: every `LayerPatch` the line produced is
//!   published through one [`Application::publish_layer_record`] with one
//!   transaction id, so undo/redo replays the whole line in one revision. Each
//!   row's validated database setter raises the revision itself (the `cad-db`
//!   rule), so a multi-row line's reported change set spans those bumps — the
//!   same documented limitation [`Application::write_layer_flags`] has;
//! * a line that changes nothing records no patch, publishes nothing (the
//!   database's own no-op rule already keeps the revision still) and reports
//!   `layer.unchanged`;
//! * `MAKE` / `SET` apply the existing active-layer admission rule
//!   ([`Application::ensure_layer_can_be_current`]) **before** any mutation, and
//!   `OFF` / `FREEZE` on the session's current layer are refused through the
//!   existing [`ensure_current_layer_stays_visible`](super::layer_edit) rule, so
//!   the current layer never becomes invisible;
//! * a created layer gets the documented default attribute row (on, thawed,
//!   unlocked), which is why `MAKE` on a missing name passes that rule;
//! * names resolve with [`resolve_layer`](super::layer_edit::resolve_layer), the
//!   same exact-then-unique case-insensitive rule the panel uses; `MAKE` asks it
//!   through `lookup_layer` so "unknown" is a decision (create) while
//!   "ambiguous" stays a refusal.
//!
//! Names in a list option are **comma-separated** (a name may contain spaces, so
//! `NEW A B` is one layer called `A B`); a layer name may not contain a comma —
//! the database refuses it (`layer.name_illegal_char`) — which is what makes the
//! list unambiguous. Whitespace around each entry is trimmed.

use super::layer_record::layer_row;
use super::{layer_edit, *};
use crate::command_payload::DashLayerOps;
use crate::layer_property::LayerPropertyValue;

impl Application {
    /// Apply one typed `-LAYER <option> …` write line.
    ///
    /// The payload arrives through [`CommandId::SetLayerProperty`], the existing
    /// Work-only layer-table carrier: the extended `-LAYER` selector is the
    /// read-only report and stays Viewer-legal, while every option here writes
    /// the drawing. See [`DashLayerOps`] for why the carrier is reused instead of
    /// adding a command.
    pub(crate) fn dash_layer_write(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::DashLayer(ops) = &command.payload else {
            return Err(CadError::InvalidInput(
                "-LAYER writes need a DashLayer payload".into(),
            ));
        };
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
        let mut patches: Vec<cad_db::LayerPatch> = Vec::new();
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        // Session effects, applied only after the candidate is published: which
        // layer becomes current, which rows were deleted (their temporary
        // overrides are pruned) and the delete fallback for the active layer.
        let mut current: Option<(LayerId, String)> = None;
        let mut deleted: Vec<LayerId> = Vec::new();
        let mut deleted_active: Option<(String, LayerId)> = None;

        match ops {
            // The list form is the Viewer-legal report, never a write. Reaching
            // here means a caller bypassed the session router; refuse by name.
            DashLayerOps::List { .. } => {
                return Err(CadError::InvalidInput(
                    "dashlayer.payload: -LAYER ? is the read-only table report and cannot be \
                     dispatched as a write"
                        .into(),
                ))
            }
            DashLayerOps::New(names) => {
                for name in names {
                    let id = create_layer_on(&mut candidate, name, transaction)?;
                    let stored = stored_name(&candidate, id, name);
                    patches.push(cad_history::layer_patch(
                        id,
                        None,
                        layer_row(&candidate, id),
                    ));
                    diagnostics.push(diagnostic(
                        "layer.created",
                        format!("-LAYER NEW: layer '{stored}' created (id {})", id.0),
                    ));
                }
            }
            DashLayerOps::Make(name) => {
                let id = match layer_edit::lookup_layer(drawing.as_ref(), name)? {
                    Some(id) => id,
                    None => {
                        let id = create_layer_on(&mut candidate, name, transaction)?;
                        let stored = stored_name(&candidate, id, name);
                        patches.push(cad_history::layer_patch(
                            id,
                            None,
                            layer_row(&candidate, id),
                        ));
                        diagnostics.push(diagnostic(
                            "layer.created",
                            format!("-LAYER MAKE: layer '{stored}' created (id {})", id.0),
                        ));
                        id
                    }
                };
                // The existing rule, before any mutation is published: a locked,
                // frozen or off existing layer is refused. A layer this line
                // creates has no stored row yet and the documented default row is
                // on/thawed/unlocked, so the same call admits it.
                self.ensure_layer_can_be_current(document_id, id)?;
                current = Some((id, stored_name(&candidate, id, name)));
            }
            DashLayerOps::Set(name) => {
                let id = layer_edit::resolve_layer(drawing.as_ref(), name)?;
                self.ensure_layer_can_be_current(document_id, id)?;
                current = Some((id, stored_name(drawing.as_ref(), id, name)));
            }
            DashLayerOps::Rename { old, new } => {
                let id = layer_edit::resolve_layer(drawing.as_ref(), old)?;
                let previous = stored_name(drawing.as_ref(), id, old);
                let before = layer_row(drawing.as_ref(), id);
                let changeset = candidate.rename_layer(id, new, transaction, "-LAYER rename")?;
                // Renaming to the stored name is an authoritative no-op: the
                // database did not bump the revision, so no patch is recorded.
                if changeset.before != changeset.after {
                    let stored = stored_name(&candidate, id, new);
                    patches.push(cad_history::layer_patch(
                        id,
                        before,
                        layer_row(&candidate, id),
                    ));
                    diagnostics.push(diagnostic(
                        "layer.renamed",
                        format!("-LAYER RENAME: layer '{previous}' renamed to '{stored}'"),
                    ));
                }
            }
            DashLayerOps::Delete(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                // Same decision as `DELETELAYER`, made before mutating: deleting
                // the active layer falls back to `0`, and a drawing without that
                // row is an explicit refusal instead of a dangling active id.
                for (id, name) in &layers {
                    if *id != session.active_layer {
                        continue;
                    }
                    let fallback = drawing
                        .layers()
                        .find(|row| row.name == cad_db::DEFAULT_LAYER_NAME)
                        .map(|row| row.id)
                        .ok_or_else(|| {
                            CadError::InvalidInput(format!(
                                "layer.default_missing: cannot delete the active layer '{name}' \
                                 because this drawing has no '{}' layer to fall back to",
                                cad_db::DEFAULT_LAYER_NAME
                            ))
                        })?;
                    deleted_active = Some((name.clone(), fallback));
                }
                for (id, name) in &layers {
                    let before = layer_row(drawing.as_ref(), *id);
                    candidate.delete_layer(*id, transaction, "-LAYER delete")?;
                    patches.push(cad_history::layer_patch(*id, before, None));
                    deleted.push(*id);
                    diagnostics.push(diagnostic(
                        "layer.deleted",
                        format!("-LAYER DELETE: layer '{name}' (id {}) deleted", id.0),
                    ));
                }
            }
            DashLayerOps::On(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    LayerPropertyValue::Off(false),
                    "off = false".to_string(),
                    "ON",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Off(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                for (id, name) in &layers {
                    layer_edit::ensure_current_layer_stays_visible(
                        session,
                        *id,
                        name,
                        &LayerPropertyValue::Off(true),
                    )?;
                }
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    LayerPropertyValue::Off(true),
                    "off = true".to_string(),
                    "OFF",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Freeze(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                for (id, name) in &layers {
                    layer_edit::ensure_current_layer_stays_visible(
                        session,
                        *id,
                        name,
                        &LayerPropertyValue::Frozen(true),
                    )?;
                }
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    LayerPropertyValue::Frozen(true),
                    "frozen = true".to_string(),
                    "FREEZE",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Thaw(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    LayerPropertyValue::Frozen(false),
                    "frozen = false".to_string(),
                    "THAW",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Lock(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    LayerPropertyValue::Locked(true),
                    "locked = true".to_string(),
                    "LOCK",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Unlock(names) => {
                let layers = resolve_names(drawing.as_ref(), names)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    LayerPropertyValue::Locked(false),
                    "locked = false".to_string(),
                    "UNLOCK",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Color { value, layers } => {
                let layers = resolve_names(drawing.as_ref(), layers)?;
                // AutoCAD's command line accepts a bare ACI number; the rewrite
                // is spelling only — the one colour grammar still decides.
                let value = crate::layer_property::normalize_color_input(value);
                let parsed = crate::layer_property::parse(drawing.as_ref(), "color", &value)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    parsed,
                    format!("color = '{value}'"),
                    "COLOR",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Linetype { name, layers } => {
                let layers = resolve_names(drawing.as_ref(), layers)?;
                let parsed = crate::layer_property::parse(drawing.as_ref(), "linetype", name)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    parsed,
                    format!("linetype = '{name}'"),
                    "LTYPE",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Lineweight { value, layers } => {
                let layers = resolve_names(drawing.as_ref(), layers)?;
                let parsed = crate::layer_property::parse(drawing.as_ref(), "lineweight", value)?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    parsed,
                    format!("lineweight = '{value}'"),
                    "LWEIGHT",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
            DashLayerOps::Plot { plottable, layers } => {
                let layers = resolve_names(drawing.as_ref(), layers)?;
                let parsed = crate::layer_property::parse(
                    drawing.as_ref(),
                    "plottable",
                    if *plottable { "true" } else { "false" },
                )?;
                write_attribute(
                    &mut candidate,
                    drawing.as_ref(),
                    &layers,
                    parsed,
                    format!("plottable = {plottable}"),
                    "PLOT",
                    transaction,
                    &mut patches,
                    &mut diagnostics,
                )?;
            }
        }

        // Publish the already-validated candidate with every patch this line
        // produced as **one** undo step, or nothing at all when no row changed.
        let changes = if patches.is_empty() {
            None
        } else {
            let changes = ChangeSet {
                database: candidate.id(),
                before: drawing.revision(),
                after: candidate.revision(),
                transaction,
                reason: "-LAYER".to_string(),
                changes: Vec::new(),
            };
            self.publish_layer_record(document_id, candidate, transaction, "-LAYER", patches)?;
            Some(Arc::new(changes))
        };

        // Session state after the publish: the drawing is authoritative and no
        // refusal can be pending any more. Deleted rows never keep a temporary
        // override (the same hygiene `DELETELAYER` does).
        for id in deleted {
            session.layer_overrides.remove(id);
        }
        if let Some((deleted_name, fallback)) = deleted_active {
            session.active_layer = fallback;
            diagnostics.push(diagnostic(
                "drawing.active_layer_fallback",
                format!(
                    "-LAYER: active layer '{deleted_name}' was deleted; the active layer is now \
                     '{}' (this session choice is not part of the layer-table undo step)",
                    cad_db::DEFAULT_LAYER_NAME
                ),
            ));
        }
        if let Some((id, name)) = current {
            if session.active_layer == id {
                diagnostics.push(diagnostic(
                    "layer.unchanged",
                    format!("-LAYER: layer '{name}' is already the current layer; nothing changed"),
                ));
            } else {
                session.active_layer = id;
                diagnostics.push(diagnostic(
                    "layer.current",
                    format!(
                        "-LAYER: layer '{name}' (id {}) is now the current layer",
                        id.0
                    ),
                ));
            }
        } else if changes.is_none() && diagnostics.is_empty() {
            diagnostics.push(diagnostic(
                "layer.unchanged",
                "-LAYER: every requested layer already had that state; nothing changed".into(),
            ));
        }

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve one comma-separated name list against the authoritative drawing.
///
/// A repeated name resolves to the same row and is dropped: one row may appear in
/// at most one [`cad_db::LayerPatch`] (the history record refuses duplicate ids),
/// and a duplicate patch could not mean anything else. Names are resolved with
/// the one [`layer_edit::resolve_layer`] rule; an unknown or ambiguous name is a
/// refusal and nothing is published.
fn resolve_names(
    database: &DrawingDatabase,
    names: &[String],
) -> CadResult<Vec<(LayerId, String)>> {
    let mut resolved: Vec<(LayerId, String)> = Vec::new();
    for name in names {
        let id = layer_edit::resolve_layer(database, name)?;
        if resolved.iter().any(|(existing, _)| *existing == id) {
            continue;
        }
        resolved.push((id, stored_name(database, id, name)));
    }
    Ok(resolved)
}

/// Create one layer on the candidate and return its id.
///
/// The name is **not** normalised here: the database trims it and refuses an
/// empty/illegal/duplicate name, so a blank entry fails explicitly instead of
/// creating something nameless.
fn create_layer_on(
    candidate: &mut DrawingDatabase,
    name: &str,
    transaction: TransactionId,
) -> CadResult<LayerId> {
    let (id, _changeset) = candidate.create_layer(name, transaction, "-LAYER new")?;
    Ok(id)
}

/// Write one already-parsed attribute value to every resolved layer.
///
/// Reads the **authoritative** attribute row for each layer (the line's rows are
/// distinct, so one pre-line read per row is the whole input), applies the value
/// to a clone and lets the database recompute `Layer::visible` for `off`/`frozen`
/// in the same mutation. A row the write did not change records no patch and a
/// `layer.unchanged` diagnostic; a changed row records exactly one patch.
#[allow(clippy::too_many_arguments)]
fn write_attribute(
    candidate: &mut DrawingDatabase,
    source: &DrawingDatabase,
    layers: &[(LayerId, String)],
    parsed: LayerPropertyValue,
    label: String,
    option: &str,
    transaction: TransactionId,
    patches: &mut Vec<cad_db::LayerPatch>,
    diagnostics: &mut Vec<Diagnostic>,
) -> CadResult<()> {
    for (id, name) in layers {
        let before = layer_row(source, *id);
        let mut attributes = source.layer_attributes(*id);
        crate::layer_property::apply(&mut attributes, parsed.clone());
        let changeset =
            candidate.set_layer_attributes(*id, attributes, transaction, "-LAYER write")?;
        if changeset.before != changeset.after {
            patches.push(cad_history::layer_patch(
                *id,
                before,
                layer_row(candidate, *id),
            ));
            diagnostics.push(diagnostic(
                "layer.property_changed",
                format!("-LAYER {option}: layer '{name}' {label}"),
            ));
        } else {
            diagnostics.push(diagnostic(
                "layer.unchanged",
                format!("-LAYER {option}: layer '{name}' already has {label}; no change"),
            ));
        }
    }
    Ok(())
}

/// The stored spelling of a layer row, or the typed name for a row that is not
/// (or no longer) there.
fn stored_name(database: &DrawingDatabase, id: LayerId, typed: &str) -> String {
    database
        .layer(id)
        .map(|row| row.name.clone())
        .unwrap_or_else(|| typed.to_string())
}

/// Build the report row with its stable code.
fn diagnostic(code: &str, message: String) -> Diagnostic {
    crate::app_reports_ext::diagnostic(code, message)
}
