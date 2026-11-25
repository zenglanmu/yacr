//! `EditorInput.editor.setSelection`: replace the session selection.
//!
//! A plugin that computes a result set (a filter, a query, a batch edit) hands
//! the ids back to the user through the host's own selection, instead of faking a
//! highlight in its own UI. The operation goes over the **mutating** lane because
//! it changes live session state, and it is answered only after the host applied
//! it.
//!
//! The request is `{ "objectIds": ["12", "13"] }`; an empty list is the real
//! "clear the selection". The host validates every id against the open drawing and
//! the active space **before** applying anything, so an unknown id is
//! `cad_api.unknown_object: <id>` and a foreign-space id is
//! `api.invalid_argument: objectIds.space: <id>` — no partial selection.
//!
//! The answer is `{ "selected": <count> }` (the ids the host accepted). It is a
//! value, not an empty success: a malformed request, a missing document and a
//! rejected id all refuse by name.

use cad_domain::ObjectId;
use serde_json::{json, Value};

use super::api_call::{
    api_call_error, api_call_invalid_argument, CadApiOutcome, API_CALL_NO_DOCUMENT,
};
use super::CadSession;

const FIELD_OBJECT_IDS: &str = "parameters.objectIds";

/// Parse the `objectIds` array. Each entry is the decimal spelling of the host's
/// `EntityId`; a non-string or non-decimal entry is refused by field.
fn parse_object_ids(parameters: Option<&Value>) -> Result<Vec<ObjectId>, String> {
    let parameters = parameters
        .filter(|parameters| parameters.is_object())
        .ok_or_else(|| api_call_invalid_argument(FIELD_OBJECT_IDS))?;
    let entries = parameters
        .get("objectIds")
        .and_then(Value::as_array)
        .ok_or_else(|| api_call_invalid_argument(FIELD_OBJECT_IDS))?;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            entry
                .as_str()
                .and_then(|value| value.parse::<u128>().ok())
                .map(ObjectId)
                .ok_or_else(|| api_call_invalid_argument(&format!("{FIELD_OBJECT_IDS}[{index}]")))
        })
        .collect()
}

impl CadSession {
    /// Answer one `EditorInput.editor.setSelection` submission.
    pub(super) fn set_selection_outcome(&self, parameters: Option<&Value>) -> CadApiOutcome {
        let ids = match parse_object_ids(parameters) {
            Ok(ids) => ids,
            Err(marker) => return CadApiOutcome::refusal(marker),
        };
        let selected = ids.len();
        let mut controller = self.core.controller.borrow_mut();
        if controller.drawing().is_none() {
            return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
        }
        match controller.plugin_select_ids(ids) {
            Ok(_) => CadApiOutcome::value(json!({ "selected": selected })),
            // The application pre-validation markers are already stable wire
            // codes; anything else is the honest generic host failure.
            Err(cad_domain::CadError::InvalidInput(message))
                if message.starts_with("cad_api.") || message.starts_with("api.") =>
            {
                CadApiOutcome::refusal(message)
            }
            Err(error) => CadApiOutcome::refusal(api_call_error(error)),
        }
    }
}
