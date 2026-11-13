//! `DatabaseServices.transaction.commit`: the one **mutating** plugin operation.
//!
//! The request is pure data (`{ document, transactionId, expectedRevision,
//! edits }`); the commit itself is the application's single API transaction path
//! ([`cad_app::HostController::plugin_commit`] → `Application::execute_api`).
//! Nothing here writes `cad-db` directly, and no second document state exists:
//! this module only maps the wire request onto [`cad_app::EntityEdit`] values and
//! maps the result back onto the frozen JSON.
//!
//! # Mapping
//!
//! * `document` must be the open document's decimal id
//!   (`api.invalid_argument: document` otherwise);
//! * `transactionId` is part of the request contract and is carried through as a
//!   required string; the host does **not** correlate by it (the transport's
//!   `requestId` is the correlation id);
//! * `expectedRevision` is the revision the caller read (`api.revision-conflict`
//!   when it is not the open revision);
//! * each `edits` element is an insert (`typeName` + `layer` + `space` +
//!   `parameters`), a translate (`objectId` + `displacement`) or a delete
//!   (`objectId`). A malformed or unresolvable field is
//!   `api.invalid_argument: <field>`; anything the application refuses after
//!   mapping is the explicit `cad_api.error: <text>`.

use cad_db::DrawingDatabase;
use cad_domain::{
    DocumentId, EntityId, InstancePath, LayerId, Point3, Revision, SelectionRef, SpaceId,
};
use serde_json::Value;

use cad_app::{EntityEdit, NewEntity};

use super::api_call::{
    api_call_error, api_call_invalid_argument, CadApiOutcome, API_CALL_NO_DOCUMENT,
    API_CALL_REVISION_CONFLICT,
};

/// The request keys, named once so a refusal marker and its field cannot drift.
const FIELD_PARAMETERS: &str = "parameters";
const FIELD_DOCUMENT: &str = "document";
const FIELD_TRANSACTION_ID: &str = "transactionId";
const FIELD_EXPECTED_REVISION: &str = "expectedRevision";
const FIELD_EDITS: &str = "edits";
const FIELD_TYPE_NAME: &str = "typeName";
const FIELD_LAYER: &str = "layer";
const FIELD_SPACE: &str = "space";
const FIELD_OBJECT_ID: &str = "objectId";
const FIELD_DISPLACEMENT: &str = "displacement";
const FIELD_START: &str = "parameters.start";
const FIELD_END: &str = "parameters.end";
const FIELD_CENTER: &str = "parameters.center";
const FIELD_RADIUS: &str = "parameters.radius";
const FIELD_START_ANGLE: &str = "parameters.startAngle";
const FIELD_END_ANGLE: &str = "parameters.endAngle";

/// The active space's wire name: `"model"` for model space, else the layout name.
///
/// This is the same vocabulary the plugin contract uses, so an insert names the
/// space it saw rather than an internal id. Block space is not a drawing space a
/// commit can target; it gets a stable, non-matching name so an insert there is
/// refused by the space rule instead of silently landing elsewhere.
pub(super) fn space_wire_name(drawing: &DrawingDatabase, space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "model".to_string(),
        SpaceId::Paper(layout) => drawing
            .layout(*layout)
            .map(|layout| layout.name.clone())
            .unwrap_or_default(),
        SpaceId::Block(block) => format!("block:{}", block.0),
    }
}

/// Resolve a layer name against the drawing's own table.
///
/// Exact match first, then a **unique** ASCII case-insensitive one — the same
/// rule the layer panel uses for its name selectors. An unknown or ambiguous
/// name is refused; no row is invented.
fn resolve_layer(drawing: &DrawingDatabase, name: &str) -> Option<LayerId> {
    if let Some(layer) = drawing.layers().find(|layer| layer.name == name) {
        return Some(layer.id);
    }
    let mut matches = drawing
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name));
    let first = matches.next()?;
    matches.next().is_none().then_some(first.id)
}

/// Parse one `[x, y, z]` triple.
fn point(value: Option<&Value>, field: &str) -> Result<Point3, String> {
    let Some(array) = value.and_then(Value::as_array) else {
        return Err(api_call_invalid_argument(field));
    };
    if array.len() != 3 {
        return Err(api_call_invalid_argument(field));
    }
    let mut coordinates = [0.0_f64; 3];
    for (slot, value) in coordinates.iter_mut().zip(array) {
        *slot = value
            .as_f64()
            .ok_or_else(|| api_call_invalid_argument(field))?;
    }
    Ok(Point3 {
        x: coordinates[0],
        y: coordinates[1],
        z: coordinates[2],
    })
}

/// Parse one scalar number.
fn number(value: Option<&Value>, field: &str) -> Result<f64, String> {
    value
        .and_then(Value::as_f64)
        .ok_or_else(|| api_call_invalid_argument(field))
}

/// Parse one decimal id (`EntityId`/`DocumentId` spelling are both decimal).
fn decimal_id(value: Option<&Value>, field: &str) -> Result<u128, String> {
    value
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<u128>().ok())
        .ok_or_else(|| api_call_invalid_argument(field))
}

/// The parsed request: the revision the caller read plus the validated edits.
pub(super) type CommitRequest = (Revision, Vec<EntityEdit>);

/// Map one `transaction.commit` parameter object onto the application request.
///
/// Every refusal is a stable `api.invalid_argument: <field>` marker; the order of
/// checks is document → transactionId → expectedRevision → edits, so the first
/// broken field is the one named.
pub(super) fn parse_commit_request(
    drawing: &DrawingDatabase,
    active_space: &SpaceId,
    document_id: DocumentId,
    parameters: Option<&Value>,
) -> Result<CommitRequest, String> {
    // The request body itself must be an object: absent, `null` and any other
    // shape name the field rather than falling through to a deeper key.
    let parameters = parameters
        .filter(|parameters| parameters.is_object())
        .ok_or_else(|| api_call_invalid_argument(FIELD_PARAMETERS))?;

    if decimal_id(parameters.get(FIELD_DOCUMENT), FIELD_DOCUMENT)? != document_id.0 {
        return Err(api_call_invalid_argument(FIELD_DOCUMENT));
    }
    // Required by the request contract; the host does not correlate by it (the
    // transport's `requestId` is the correlation id).
    parameters
        .get(FIELD_TRANSACTION_ID)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| api_call_invalid_argument(FIELD_TRANSACTION_ID))?;
    let expected_revision = parameters
        .get(FIELD_EXPECTED_REVISION)
        .and_then(Value::as_u64)
        .ok_or_else(|| api_call_invalid_argument(FIELD_EXPECTED_REVISION))?;

    let edits = parameters
        .get(FIELD_EDITS)
        .and_then(Value::as_array)
        .ok_or_else(|| api_call_invalid_argument(FIELD_EDITS))?;
    let space_name = space_wire_name(drawing, active_space);
    let mut mapped = Vec::with_capacity(edits.len());
    for edit in edits {
        mapped.push(map_edit(
            drawing,
            active_space,
            document_id,
            &space_name,
            edit,
        )?);
    }
    Ok((Revision(expected_revision), mapped))
}

/// Map one edit object.
fn map_edit(
    drawing: &DrawingDatabase,
    active_space: &SpaceId,
    document_id: DocumentId,
    space_name: &str,
    edit: &Value,
) -> Result<EntityEdit, String> {
    if edit.get(FIELD_TYPE_NAME).is_some() {
        let type_name = edit
            .get(FIELD_TYPE_NAME)
            .and_then(Value::as_str)
            .ok_or_else(|| api_call_invalid_argument(FIELD_TYPE_NAME))?;
        let layer_name = edit
            .get(FIELD_LAYER)
            .and_then(Value::as_str)
            .ok_or_else(|| api_call_invalid_argument(FIELD_LAYER))?;
        let layer = resolve_layer(drawing, layer_name)
            .ok_or_else(|| api_call_invalid_argument(FIELD_LAYER))?;
        let requested_space = edit
            .get(FIELD_SPACE)
            .and_then(Value::as_str)
            .ok_or_else(|| api_call_invalid_argument(FIELD_SPACE))?;
        if requested_space != space_name {
            return Err(api_call_invalid_argument(FIELD_SPACE));
        }
        let entity = map_new_entity(type_name, edit.get(FIELD_PARAMETERS))?;
        return Ok(EntityEdit::Insert {
            layer,
            space: active_space.clone(),
            entity,
        });
    }
    if edit.get(FIELD_DISPLACEMENT).is_some() {
        let object = decimal_id(edit.get(FIELD_OBJECT_ID), FIELD_OBJECT_ID)?;
        let delta = point(edit.get(FIELD_DISPLACEMENT), FIELD_DISPLACEMENT)?;
        return Ok(EntityEdit::Translate {
            reference: reference(document_id, object),
            delta,
        });
    }
    if edit.get(FIELD_OBJECT_ID).is_some() {
        let object = decimal_id(edit.get(FIELD_OBJECT_ID), FIELD_OBJECT_ID)?;
        return Ok(EntityEdit::Delete {
            reference: reference(document_id, object),
        });
    }
    Err(api_call_invalid_argument(FIELD_EDITS))
}

/// A top-level entity reference (no instance path, no sub-element).
fn reference(document_id: DocumentId, entity: u128) -> SelectionRef {
    SelectionRef {
        document: document_id,
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// Map a `typeName` + `parameters` pair onto [`NewEntity`].
///
/// Angles are radians (the database's own unit); every geometry field is
/// required and refused by name when absent or malformed.
fn map_new_entity(type_name: &str, parameters: Option<&Value>) -> Result<NewEntity, String> {
    let field = |name: &str| parameters.and_then(|parameters| parameters.get(name));
    match type_name {
        "Line" => Ok(NewEntity::Line {
            start: point(field("start"), FIELD_START)?,
            end: point(field("end"), FIELD_END)?,
        }),
        "Circle" => Ok(NewEntity::Circle {
            center: point(field("center"), FIELD_CENTER)?,
            radius: number(field("radius"), FIELD_RADIUS)?,
        }),
        "Arc" => Ok(NewEntity::Arc {
            center: point(field("center"), FIELD_CENTER)?,
            radius: number(field("radius"), FIELD_RADIUS)?,
            start_angle: number(field("startAngle"), FIELD_START_ANGLE)?,
            end_angle: number(field("endAngle"), FIELD_END_ANGLE)?,
        }),
        _ => Err(api_call_invalid_argument(FIELD_TYPE_NAME)),
    }
}

/// Answer one `transaction.commit` submission.
impl super::CadSession {
    pub(super) fn commit_outcome(&self, parameters: Option<&Value>) -> CadApiOutcome {
        let mut controller = self.core.controller.borrow_mut();
        let Some(drawing) = controller.drawing() else {
            return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
        };
        let (expected_revision, edits) = match parse_commit_request(
            drawing.as_ref(),
            &controller.active_space(),
            controller.document_id,
            parameters,
        ) {
            Ok(parsed) => parsed,
            Err(marker) => return CadApiOutcome::refusal(marker),
        };
        match controller.plugin_commit(expected_revision, edits) {
            Ok(outcome) => match controller.drawing() {
                Some(drawing) => CadApiOutcome::value(serde_json::json!({
                    "revision": drawing.revision().0,
                    "created": outcome
                        .created
                        .iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>(),
                })),
                // A successful commit always leaves a document; if it did not,
                // say so instead of reporting a revision the session cannot back.
                None => CadApiOutcome::refusal(API_CALL_NO_DOCUMENT),
            },
            // A stale expected revision is the contract's named conflict; any
            // other application refusal keeps its own text.
            Err(cad_domain::CadError::StaleResult) => {
                CadApiOutcome::refusal(API_CALL_REVISION_CONFLICT)
            }
            Err(error) => CadApiOutcome::refusal(api_call_error(error)),
        }
    }
}
