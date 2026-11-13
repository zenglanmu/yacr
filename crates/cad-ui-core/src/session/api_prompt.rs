//! The plugin `Editor` prompt ops: parameter parsing, routing and the answers.
//!
//! A prompt starts on the submit lane and completes on a later poll: the input
//! that answers it is an ordinary interaction (a click, typed text, Enter, Esc)
//! resolved by [`cad_app::plugin_prompt`], and the answer is taken from the
//! controller by [`CadSession::api_prompt_results`] on the owner tick.
//!
//! # Wire shape
//!
//! The client's `PromptResult` (`packages/cad-plugin-api/src/editor-input.ts`) is
//! the contract, so the `status` is the PascalCase `Ok`/`Cancel`/`None`/`Keyword`
//! and the value fields are the ones `decodeObject`'s prompt readers take:
//!
//! | operation | answer |
//! |---|---|
//! | `editor.getPoint` | `{status:"Ok", point:{x,y,z}}` |
//! | `editor.getEntity` | `{status:"Ok", objectId:"<decimal>"}` |
//! | `editor.getSelection` | `{status:"Ok", objectIds:[…]}` |
//! | `editor.getString` | `{status:"Ok", value:"…"}` |
//! | `editor.getDouble` / `getInteger` | `{status:"Ok", value:<number>}` |
//! | `editor.getKeyword` | `{status:"Keyword", keyword:"…"}` |
//! | any prompt, Esc | `{status:"Cancel"}` |
//! | any prompt, Enter with no default | `{status:"None"}` |
//! | `editor.selectAll` | `{status:"Ok", objectIds:[…]}` (immediate) |
//!
//! A keyword answer is `Keyword` rather than `Ok` because that is the status the
//! client's decoder keys on; the keyword itself is a matched entry of the
//! prompt's own list, never a coerced value.

use serde::Serialize;
use serde_json::{json, Value};

use cad_app::plugin_prompt::{ApiPrompt, ApiPromptDefault, ApiPromptKind, ApiPromptReply};

use super::api_call::{
    api_call_invalid_argument, api_call_unknown, validate_request_id, CadApiOutcome,
    CadApiSubmission, API_CALL_EDITOR_NAMESPACE, API_CALL_NO_DOCUMENT, API_SUBMIT_INVALID_REQUEST,
};

/// The prompt operations this build serves on the submit lane.
pub const API_CALL_GET_POINT: &str = "editor.getPoint";
pub const API_CALL_GET_ENTITY: &str = "editor.getEntity";
pub const API_CALL_GET_SELECTION: &str = "editor.getSelection";
pub const API_CALL_GET_STRING: &str = "editor.getString";
pub const API_CALL_GET_DOUBLE: &str = "editor.getDouble";
pub const API_CALL_GET_INTEGER: &str = "editor.getInteger";
pub const API_CALL_GET_KEYWORD: &str = "editor.getKeyword";

/// The immediate selection read (`SELECTALL`), also on the submit lane.
pub const API_CALL_SELECT_ALL: &str = "editor.selectAll";

/// The pending-prompt kind on the wire (`pendingPrompt.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CadPendingPromptKind {
    Point,
    Entity,
    Selection,
    String,
    Double,
    Integer,
    Keyword,
}

impl CadPendingPromptKind {
    fn from_plugin(kind: ApiPromptKind) -> Self {
        match kind {
            ApiPromptKind::Point => Self::Point,
            ApiPromptKind::Entity => Self::Entity,
            ApiPromptKind::Selection => Self::Selection,
            ApiPromptKind::String => Self::String,
            ApiPromptKind::Double => Self::Double,
            ApiPromptKind::Integer => Self::Integer,
            ApiPromptKind::Keyword => Self::Keyword,
        }
    }
}

/// The frozen `CadStatus.pendingPrompt` block.
///
/// `error` carries the last invalid-input sentence while the prompt stays
/// pending (re-prompt semantics); it is `null` otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadPendingPrompt {
    /// Which value the pending prompt asks for.
    pub kind: CadPendingPromptKind,
    /// The sentence shown to the user.
    pub message: String,
    /// The accepted keywords, in the caller's spelling.
    pub keywords: Vec<String>,
    /// The last invalid-input sentence, or `null`.
    pub error: Option<String>,
}

/// How one `EditorInput` submission resolved.
pub(super) enum EditorSubmit {
    /// Not an `EditorInput` operation: the caller routes it normally.
    NotEditor,
    /// A prompt started; its answer arrives on a later poll.
    Started,
    /// A settled answer (an immediate read or a refusal).
    Settled(Box<CadApiOutcome>),
}

/// The prompt kind one operation starts, when it is a prompt op.
fn prompt_kind(operation: &str) -> Option<ApiPromptKind> {
    match operation {
        API_CALL_GET_POINT => Some(ApiPromptKind::Point),
        API_CALL_GET_ENTITY => Some(ApiPromptKind::Entity),
        API_CALL_GET_SELECTION => Some(ApiPromptKind::Selection),
        API_CALL_GET_STRING => Some(ApiPromptKind::String),
        API_CALL_GET_DOUBLE => Some(ApiPromptKind::Double),
        API_CALL_GET_INTEGER => Some(ApiPromptKind::Integer),
        API_CALL_GET_KEYWORD => Some(ApiPromptKind::Keyword),
        _ => None,
    }
}

/// The client's `PromptResult` JSON for one settled reply.
fn reply_value(reply: &ApiPromptReply) -> Value {
    match reply {
        ApiPromptReply::Point(point) => json!({
            "status": "Ok",
            "point": { "x": point.x, "y": point.y, "z": point.z },
        }),
        ApiPromptReply::Entity(id) => json!({
            "status": "Ok",
            "objectId": id.0.to_string(),
        }),
        ApiPromptReply::Selection(ids) => json!({
            "status": "Ok",
            "objectIds": ids.iter().map(|id| id.0.to_string()).collect::<Vec<_>>(),
        }),
        ApiPromptReply::String(text) => json!({ "status": "Ok", "value": text }),
        ApiPromptReply::Number(value) => json!({ "status": "Ok", "value": value }),
        ApiPromptReply::Keyword(keyword) => json!({
            "status": "Keyword",
            "keyword": keyword,
        }),
        ApiPromptReply::None => json!({ "status": "None" }),
        ApiPromptReply::Cancel => json!({ "status": "Cancel" }),
    }
}

/// The request body must be an object (absent/`null`/other names `parameters`).
fn request_object(parameters: Option<&Value>) -> Result<&Value, String> {
    parameters
        .filter(|parameters| parameters.is_object())
        .ok_or_else(|| api_call_invalid_argument("parameters"))
}

/// A `message`/`rejectMessage` string, when present.
fn optional_text(parameters: &Value, field: &str) -> Result<Option<String>, String> {
    match parameters.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(api_call_invalid_argument(field)),
    }
}

/// The `keywords` list, when present (every entry a non-empty string).
fn keywords(parameters: &Value, required: bool) -> Result<Vec<String>, String> {
    let Some(value) = parameters.get("keywords") else {
        if required {
            return Err(api_call_invalid_argument("keywords"));
        }
        return Ok(Vec::new());
    };
    let Some(entries) = value.as_array() else {
        return Err(api_call_invalid_argument("keywords"));
    };
    if required && entries.is_empty() {
        return Err(api_call_invalid_argument("keywords"));
    }
    let mut keywords = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(keyword) = entry.as_str().filter(|keyword| !keyword.trim().is_empty()) else {
            return Err(api_call_invalid_argument("keywords"));
        };
        keywords.push(keyword.to_string());
    }
    Ok(keywords)
}

/// The Enter default, from `default` or the client's own `defaultValue` spelling.
///
/// `getString` takes a string, the numeric prompts a finite number; `getInteger`
/// also requires it to be whole (the client reads the answer as a safe integer).
fn default_value(
    parameters: &Value,
    kind: ApiPromptKind,
) -> Result<Option<ApiPromptDefault>, String> {
    let Some(value) = parameters
        .get("default")
        .or_else(|| parameters.get("defaultValue"))
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    match kind {
        ApiPromptKind::String => value
            .as_str()
            .map(|text| ApiPromptDefault::String(text.to_string()))
            .map(Some)
            .ok_or_else(|| api_call_invalid_argument("default")),
        ApiPromptKind::Double | ApiPromptKind::Integer => {
            let number = value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| api_call_invalid_argument("default"))?;
            if kind == ApiPromptKind::Integer && number.fract() != 0.0 {
                return Err(api_call_invalid_argument("default"));
            }
            Ok(Some(ApiPromptDefault::Number(number)))
        }
        _ => Err(api_call_invalid_argument("default")),
    }
}

/// Build one prompt from its submit parameters.
pub(super) fn parse_prompt(
    request_id: &str,
    kind: ApiPromptKind,
    parameters: Option<&Value>,
) -> Result<ApiPrompt, String> {
    let parameters = request_object(parameters)?;
    let message = optional_text(parameters, "message")?;
    let mut prompt = ApiPrompt::new(request_id, kind, message);
    prompt.keywords = keywords(parameters, kind == ApiPromptKind::Keyword)?;
    prompt.default = default_value(parameters, kind)?;
    prompt.reject_message = optional_text(parameters, "rejectMessage")?;
    Ok(prompt)
}

impl super::CadSession {
    /// The pending prompt, for the `cad_status` projection.
    pub fn pending_prompt(&self) -> Option<CadPendingPrompt> {
        let controller = self.core.controller.borrow();
        let prompt = controller.api_prompt()?;
        Some(CadPendingPrompt {
            kind: CadPendingPromptKind::from_plugin(prompt.kind),
            message: prompt.message.clone(),
            keywords: prompt.keywords.clone(),
            error: controller.api_prompt_error().map(str::to_string),
        })
    }

    /// Take every completed prompt answer, oldest first, as wire outcomes.
    pub fn api_prompt_results(&self) -> Vec<CadApiOutcome> {
        let mut controller = self.core.controller.borrow_mut();
        controller
            .take_api_prompt_results()
            .into_iter()
            .map(|result| {
                CadApiOutcome::value(reply_value(&result.reply)).with_request_id(result.request_id)
            })
            .collect()
    }

    /// Route one `EditorInput` submission.
    pub(super) fn editor_submit(&self, submission: &CadApiSubmission) -> EditorSubmit {
        if submission.namespace != API_CALL_EDITOR_NAMESPACE {
            return EditorSubmit::NotEditor;
        }
        if validate_request_id(&submission.request_id).is_err() {
            return EditorSubmit::Settled(Box::new(
                CadApiOutcome::refusal(API_SUBMIT_INVALID_REQUEST)
                    .with_request_id(submission.request_id.clone()),
            ));
        }
        if submission.operation == API_CALL_SELECT_ALL {
            let outcome = self
                .select_all_outcome(submission.parameters.as_ref())
                .with_request_id(submission.request_id.clone());
            return EditorSubmit::Settled(Box::new(outcome));
        }
        let Some(kind) = prompt_kind(&submission.operation) else {
            return EditorSubmit::Settled(Box::new(
                CadApiOutcome::refusal(api_call_unknown(
                    &submission.namespace,
                    &submission.operation,
                ))
                .with_request_id(submission.request_id.clone()),
            ));
        };
        match self.begin_prompt_outcome(submission, kind) {
            Ok(()) => EditorSubmit::Started,
            Err(marker) => EditorSubmit::Settled(Box::new(
                CadApiOutcome::refusal(marker).with_request_id(submission.request_id.clone()),
            )),
        }
    }

    /// Parse and start one prompt; `Err` carries the refusal marker.
    fn begin_prompt_outcome(
        &self,
        submission: &CadApiSubmission,
        kind: ApiPromptKind,
    ) -> Result<(), String> {
        let mut controller = self.core.controller.borrow_mut();
        if controller.drawing().is_none() {
            return Err(API_CALL_NO_DOCUMENT.to_string());
        }
        let prompt = parse_prompt(&submission.request_id, kind, submission.parameters.as_ref())?;
        controller
            .begin_api_prompt(prompt)
            // The refusals are the stable markers; a host error keeps its text.
            .map_err(|error| match error {
                cad_domain::CadError::InvalidInput(marker) => marker,
                other => other.to_string(),
            })
    }

    /// Run the existing `SELECTALL` path and answer the ids it published.
    fn select_all_outcome(&self, parameters: Option<&Value>) -> CadApiOutcome {
        if let Err(marker) = request_object(parameters) {
            return CadApiOutcome::refusal(marker);
        }
        let mut controller = self.core.controller.borrow_mut();
        if controller.drawing().is_none() {
            return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
        }
        match controller.plugin_select_all() {
            Ok(ids) => CadApiOutcome::value(reply_value(&ApiPromptReply::Selection(ids))),
            Err(error) => CadApiOutcome::refusal(super::api_call::api_call_error(error)),
        }
    }
}
