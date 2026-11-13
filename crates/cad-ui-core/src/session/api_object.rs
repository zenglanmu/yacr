//! The session-bound object reads: `database.getObjectId` and
//! `transaction.getObject`.
//!
//! Both are **submit-lane** operations (`cad_api_submit` → `cad_api_poll`): the
//! answer is read from the live session, not from the reflected snapshot, and the
//! client polls for it. The synchronous read lane keeps refusing them (a
//! live-session read is not a snapshot projection).
//!
//! # What the client decodes
//!
//! `transaction.getObject` answers exactly the object `decodeObject` in
//! `packages/cad-plugin-api` accepts: `objectId`, `handle` (`source_handle` or
//! `null` — never synthesized), `typeName`, `layer` (a **name**), `color`
//! (`EntityColor`), `linetype` (`LinetypeRef`), plus the shape's own fields
//! (`startPoint`/`endPoint`/`length`, `center`/`radius`/…, `textString`/…).
//! Lengths are computed here (segment distance, circumference, arc length) — real
//! values, not guesses.
//!
//! A geometry this build has no client shape for is refused by **name**
//! (`unsupported-runtime: cad_api.DatabaseServices.transaction.getObject:
//! <variant>`), never flattened into invented fields. `Polyline` is refused for
//! that reason today: the shipped `decodePolyline` reads a per-point field name no
//! point record can carry, so no answer this host could send would decode; fixing
//! that is a `packages/cad-plugin-api` change, not a host one.
//!
//! # No transaction is opened
//!
//! `transactionId` and `mode` are validated as the contract requires but do not
//! open a host transaction: the client buffers its own unit of work and the one
//! write is `transaction.commit`. Nothing here writes `cad-db`.

use cad_db::{DbEntity, DrawingDatabase, EntityColor as DbEntityColor, EntityLineType};
use cad_domain::{EntityId, Point3, SemanticGeometry};
use serde_json::{json, Value};

use super::api_call::{
    api_call_invalid_argument, api_call_unknown_object, api_call_unsupported_variant,
    CadApiOutcome, API_CALL_DATABASE_NAMESPACE, API_CALL_GET_OBJECT, API_CALL_NO_DOCUMENT,
};

/// The request fields, named once so a refusal marker and its field cannot drift.
const FIELD_PARAMETERS: &str = "parameters";
const FIELD_DATABASE: &str = "database";
const FIELD_HANDLE: &str = "handle";
const FIELD_DOCUMENT: &str = "document";
const FIELD_TRANSACTION_ID: &str = "transactionId";
const FIELD_OBJECT_ID: &str = "objectId";
const FIELD_MODE: &str = "mode";

/// The open modes the contract declares.
const OPEN_MODES: [&str; 3] = ["read", "write", "notify"];

/// The request body must be an object (absent/`null`/other names `parameters`).
fn request_object(parameters: Option<&Value>) -> Result<&Value, String> {
    parameters
        .filter(|parameters| parameters.is_object())
        .ok_or_else(|| api_call_invalid_argument(FIELD_PARAMETERS))
}

/// A non-empty string argument, or the field's stable refusal.
fn required_text<'a>(parameters: &'a Value, field: &str) -> Result<&'a str, String> {
    parameters
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| api_call_invalid_argument(field))
}

/// Whether `handle` is the uppercase-hex spelling the contract names.
///
/// The comparison against the database is case-insensitive, but a value that is
/// not hexadecimal at all is a caller error rather than "no such entity".
fn is_hex_handle(handle: &str) -> bool {
    !handle.is_empty() && handle.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The request must name the open database.
fn require_database(drawing: &DrawingDatabase, parameters: &Value) -> Result<(), String> {
    if required_text(parameters, FIELD_DATABASE)? != drawing.id().0.to_string() {
        return Err(api_call_invalid_argument(FIELD_DATABASE));
    }
    Ok(())
}

/// Answer one submit-lane object request.
impl super::CadSession {
    /// `DatabaseServices.database.getObjectId`.
    pub(super) fn get_object_id_outcome(&self, parameters: Option<&Value>) -> CadApiOutcome {
        let controller = self.core.controller.borrow();
        let Some(drawing) = controller.drawing() else {
            return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
        };
        let parameters = match request_object(parameters) {
            Ok(parameters) => parameters,
            Err(marker) => return CadApiOutcome::refusal(marker),
        };
        if let Err(marker) = require_database(drawing.as_ref(), parameters) {
            return CadApiOutcome::refusal(marker);
        }
        let handle = match required_text(parameters, FIELD_HANDLE) {
            Ok(handle) if is_hex_handle(handle) => handle,
            _ => return CadApiOutcome::refusal(api_call_invalid_argument(FIELD_HANDLE)),
        };
        CadApiOutcome::value(json!({ "objectId": find_object_id(drawing.as_ref(), handle) }))
    }

    /// `DatabaseServices.transaction.getObject`.
    pub(super) fn get_object_outcome(&self, parameters: Option<&Value>) -> CadApiOutcome {
        let controller = self.core.controller.borrow();
        let Some(drawing) = controller.drawing() else {
            return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
        };
        let parameters = match request_object(parameters) {
            Ok(parameters) => parameters,
            Err(marker) => return CadApiOutcome::refusal(marker),
        };
        let document_ok = required_text(parameters, FIELD_DOCUMENT)
            .map(|document| document == controller.document_id.0.to_string())
            .unwrap_or(false);
        if !document_ok {
            return CadApiOutcome::refusal(api_call_invalid_argument(FIELD_DOCUMENT));
        }
        // Required by the contract; the client owns its own unit of work, so the
        // host opens no transaction here (see the module docs).
        if let Err(marker) = required_text(parameters, FIELD_TRANSACTION_ID) {
            return CadApiOutcome::refusal(marker);
        }
        let requested = match required_text(parameters, FIELD_OBJECT_ID).and_then(|text| {
            text.parse::<u128>()
                .map_err(|_| api_call_invalid_argument(FIELD_OBJECT_ID))
        }) {
            Ok(id) => id,
            Err(marker) => return CadApiOutcome::refusal(marker),
        };
        match required_text(parameters, FIELD_MODE) {
            Ok(mode) if OPEN_MODES.contains(&mode) => {}
            _ => return CadApiOutcome::refusal(api_call_invalid_argument(FIELD_MODE)),
        }
        let Some(entity) = drawing.entity(EntityId(requested)) else {
            return CadApiOutcome::refusal(api_call_unknown_object(&requested.to_string()));
        };
        match entity_value(drawing.as_ref(), entity) {
            Ok(value) => CadApiOutcome::value(json!({ "object": value })),
            Err(marker) => CadApiOutcome::refusal(marker),
        }
    }
}

/// The decimal id of the entity whose imported handle matches, case-insensitively.
///
/// **Linear scan**: the database keeps no handle index (a handle is import
/// provenance, not an identity), and a plugin lookup is a rare, caller-driven read
/// — never a per-frame path. An unknown handle is `None`, which the caller answers
/// as the real `{ "objectId": null }`.
pub(super) fn find_object_id(drawing: &DrawingDatabase, handle: &str) -> Option<String> {
    drawing
        .entities()
        .find(|entity| {
            entity
                .object
                .source_handle
                .as_deref()
                .is_some_and(|source| source.eq_ignore_ascii_case(handle))
        })
        .map(|entity| entity.id.0.to_string())
}

/// One entity as the client's `decodeObject` reads it.
pub(super) fn entity_value(drawing: &DrawingDatabase, entity: &DbEntity) -> Result<Value, String> {
    let attributes = drawing.entity_render_attributes(entity.id);
    let layer = drawing
        .layer(entity.layer)
        .map(|layer| layer.name.clone())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            api_call_unsupported_variant(API_CALL_DATABASE_NAMESPACE, API_CALL_GET_OBJECT, "layer")
        })?;
    let mut value = geometry_value(&entity.geometry)?;
    let Some(object) = value.as_object_mut() else {
        // `geometry_value` always builds an object; a violated invariant is named.
        return Err(api_call_unsupported_variant(
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_GET_OBJECT,
            "object",
        ));
    };
    object.insert("objectId".to_string(), json!(entity.id.0.to_string()));
    // Import provenance only: a host-created entity has none, and this never
    // mints one.
    object.insert(
        "handle".to_string(),
        json!(entity.object.source_handle.as_deref()),
    );
    object.insert("layer".to_string(), json!(layer));
    object.insert("color".to_string(), entity_color(&attributes.color));
    object.insert(
        "linetype".to_string(),
        entity_linetype(&attributes.linetype)?,
    );
    Ok(value)
}

/// The client's `EntityColor`: the method tag plus its own value.
fn entity_color(color: &DbEntityColor) -> Value {
    match color {
        DbEntityColor::Explicit([red, green, blue]) => json!({
            "method": "byColor",
            "rgb": format!("#{red:02x}{green:02x}{blue:02x}"),
        }),
        DbEntityColor::ByBlock => json!({ "method": "byBlock" }),
        DbEntityColor::ByLayer => json!({ "method": "byLayer" }),
    }
}

/// The client's `LinetypeRef`: symbolic, or the resolved pattern name.
///
/// An explicit linetype with a blank stored name cannot be reported as `named`
/// (the client refuses a blank name) and is not honestly `byLayer`, so it is
/// refused by name instead of guessed.
fn entity_linetype(linetype: &EntityLineType) -> Result<Value, String> {
    match linetype {
        EntityLineType::Explicit { name, .. } if !name.trim().is_empty() => {
            Ok(json!({ "method": "named", "name": name }))
        }
        EntityLineType::Explicit { .. } => Err(api_call_unsupported_variant(
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_GET_OBJECT,
            "linetype",
        )),
        EntityLineType::ByBlock => Ok(json!({ "method": "byBlock" })),
        EntityLineType::ByLayer => Ok(json!({ "method": "byLayer" })),
    }
}

/// The geometry's client shape (`typeName` plus the shape's own fields).
///
/// Only the shapes `decodeObject` can consume are emitted; every other variant is
/// refused by name.
fn geometry_value(geometry: &SemanticGeometry) -> Result<Value, String> {
    match geometry {
        SemanticGeometry::Line { start, end } => Ok(json!({
            "typeName": "Line",
            "startPoint": point(start),
            "endPoint": point(end),
            "length": distance(start, end),
        })),
        SemanticGeometry::Circle { center, radius, .. } => Ok(json!({
            "typeName": "Circle",
            "center": point(center),
            "radius": radius,
            "length": std::f64::consts::TAU * radius.abs(),
        })),
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => Ok(json!({
            "typeName": "Arc",
            "center": point(center),
            "radius": radius,
            // Radians, counter-clockwise from +X: the stored start and its sweep
            // end (never normalized or reversed).
            "startAngle": start,
            "endAngle": start + sweep,
            "length": radius.abs() * sweep.abs(),
        })),
        SemanticGeometry::Text {
            text,
            position,
            height,
            rotation,
            ..
        } => Ok(json!({
            "typeName": "DBText",
            "textString": text,
            "position": point(position),
            "height": height,
            "rotation": rotation,
        })),
        other => Err(api_call_unsupported_variant(
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_GET_OBJECT,
            geometry_variant_name(other),
        )),
    }
}

/// The stable name of a geometry variant for a refusal marker.
fn geometry_variant_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "Line",
        SemanticGeometry::Polyline { .. } => "Polyline",
        SemanticGeometry::Circle { .. } => "Circle",
        SemanticGeometry::Arc { .. } => "Arc",
        SemanticGeometry::Ellipse { .. } => "Ellipse",
        SemanticGeometry::Spline { .. } => "Spline",
        SemanticGeometry::Point(_) => "Point",
        SemanticGeometry::Mesh(_) => "Mesh",
        SemanticGeometry::Insert { .. } => "Insert",
        SemanticGeometry::Text { .. } => "DBText",
        SemanticGeometry::Shape { .. } => "Shape",
        SemanticGeometry::Opaque { .. } => "Opaque",
        SemanticGeometry::Compound(_) => "Compound",
        SemanticGeometry::Dimension(_) => "Dimension",
        SemanticGeometry::Hatch(_) => "Hatch",
        SemanticGeometry::MultiLeader(_) => "MultiLeader",
        SemanticGeometry::Table(_) => "Table",
        SemanticGeometry::Image { .. } => "Image",
        SemanticGeometry::Mask { .. } => "Mask",
    }
}

/// A 3D point in the wire's `[x, y, z]` form (the symbol-table `origin` form).
fn point(point: &Point3) -> Value {
    json!([point.x, point.y, point.z])
}

/// The segment length between two points.
fn distance(start: &Point3, end: &Point3) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let dz = end.z - start.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}
